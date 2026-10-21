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
