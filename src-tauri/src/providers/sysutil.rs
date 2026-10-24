//! Win32 进程/窗口小工具，供剪贴板来源检测与窗口切换器共用

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

/// pid → 进程名词缓存：窗口切换器每次击键都逐窗口取进程名，
/// OpenProcess + QueryFullProcessImageName 不便宜；pid 的进程名终身不变，
/// 查询结果缓存即可。超上限整体清空，防 pid 复用后映射长期陈旧。
static PID_STEMS: LazyLock<Mutex<HashMap<u32, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn hwnd_pid(hwnd: HWND) -> Option<u32> {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    (pid != 0).then_some(pid)
}

pub fn process_path_of_pid(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        if QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok()
        {
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        } else {
            None
        }
    }
}

pub fn process_stem_of_pid(pid: u32) -> Option<String> {
    const CACHE_CAP: usize = 4096;
    let mut cache = PID_STEMS.lock().expect("pid stems poisoned");
    if cache.len() > CACHE_CAP {
        cache.clear();
    }
    if let Some(stem) = cache.get(&pid) {
        return Some(stem.clone());
    }
    let stem = process_path_of_pid(pid).and_then(|path| {
        Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
    });
    if let Some(s) = &stem {
        cache.insert(pid, s.clone());
    }
    stem
}

/// 以管理员身份运行（ShellExecuteW "runas"，弹一次 UAC；取消/失败返回 Err）
pub fn run_as_admin(path: &str) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;

    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let file: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let h = unsafe {
        ShellExecuteW(
            Some(HWND::default()),
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // 返回值 > 32 为成功（<=32 是 SE_ERR 错误码）
    if h.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!(
            "管理员启动失败（ShellExecute 代码 {}）",
            h.0 as isize
        ))
    }
}

/// 资源管理器中定位并选中文件（explorer /select）。路径不存在时报错而非静默打开别处
pub fn reveal_in_explorer(path: &str) -> Result<(), String> {
    if !Path::new(path).exists() {
        return Err("文件已不存在或被移动".into());
    }
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(windows)]
    {
        // /select 与带空格路径必须同参（explorer 的古怪解析），raw_arg 控制引号原样下发
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(format!("/select,\"{}\"", path));
    }
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}
