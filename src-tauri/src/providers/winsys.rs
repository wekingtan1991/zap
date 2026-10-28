//! 窗口切换器：枚举可见顶级窗口，搜索后置前

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
    IsWindowVisible, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use super::SearchItem;

/// 窗口快照的复用窗口：连续击键共用一次枚举结果（EnumWindows + 逐窗口
/// 进程路径解析不便宜），过期后下次搜索重建。激活用的是 hwnd，快照期间
/// 窗口关闭只导致 SetForegroundWindow 空打，无副作用。
const SNAPSHOT_TTL: Duration = Duration::from_millis(1500);

static SNAPSHOT: Mutex<Option<(Instant, Arc<Vec<WinEntry>>)>> = Mutex::new(None);

fn snapshot() -> Arc<Vec<WinEntry>> {
    let mut guard = SNAPSHOT.lock().expect("win snapshot poisoned");
    if let Some((at, list)) = guard.as_ref()
        && at.elapsed() < SNAPSHOT_TTL
    {
        return list.clone();
    }
    let list = Arc::new(enumerate());
    *guard = Some((Instant::now(), list.clone()));
    list
}

#[derive(Clone)]
pub struct WinEntry {
    pub hwnd: isize,
    pub title: String,
    pub process: String,
    /// "标题 进程名" 的匹配形态（小写/拼音），枚举时算一次
    forms: super::MatchForms,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let out = &mut *(lparam.0 as *mut Vec<WinEntry>);
        if !IsWindowVisible(hwnd).as_bool() {
            return true.into();
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return true.into();
        }
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..n as usize]);

        // UWP 挂起窗口是 cloaked 状态，视觉上不存在，跳过
        let mut cloaked = 0u32;
        let is_cloaked = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0;

        // 工具窗口（托盘浮层之类）跳过
        let exstyle = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let is_tool = (exstyle as u32 & WS_EX_TOOLWINDOW.0) != 0;

        if is_cloaked || is_tool {
            return true.into();
        }

        let pid = super::sysutil::hwnd_pid(hwnd);
        // 跳过自己
        if pid == Some(std::process::id()) {
            return true.into();
        }
        let process = pid
            .and_then(super::sysutil::process_stem_of_pid)
            .unwrap_or_default();
        let forms = super::MatchForms::new(&format!("{title} {process}"));
        out.push(WinEntry {
            hwnd: hwnd.0 as isize,
            title,
            process,
            forms,
        });
        true.into()
    }
}

pub fn enumerate() -> Vec<WinEntry> {
    let mut list: Vec<WinEntry> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

pub fn search(query: &str, limit: usize) -> Vec<SearchItem> {
    let q = query.trim().to_lowercase();
    let entries = snapshot();
    let mut scored: Vec<(f32, &WinEntry)> = entries
        .iter()
        .filter_map(|e| e.forms.score(&q).map(|s| (s, e)))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);

    scored
        .into_iter()
        .enumerate()
        .map(|(i, (score, e))| SearchItem {
            id: format!("win:{:x}", e.hwnd),
            title: e.title.clone(),
            subtitle: e.process.clone(),
            kind: "win".into(),
            target: e.hwnd.to_string(),
            icon: None,
            pinned: false,
            score: score * 0.55 - i as f32,
        })
        .collect()
}

/// 把窗口拉到前台（最小化则先还原）。热键触发时系统会授予前台切换权限。
pub fn activate(hwnd_val: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    let hwnd = HWND(hwnd_val as *mut core::ffi::c_void);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd).as_bool()
    }
}

/// 焦点交还呼出前的窗口并注入 Ctrl+V（剪贴板「选中即粘贴」）。
/// 面板刚隐藏时本进程尚在前台，有权指定下一个前台窗口；重试至多 ~600ms
/// 等目标窗口真正到前台，超时返回 false（调用方保持仅回填的行为）。
pub fn paste_to_prev(prev_hwnd: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    let hwnd = HWND(prev_hwnd as *mut core::ffi::c_void);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let deadline = Instant::now() + Duration::from_millis(600);
        loop {
            let _ = SetForegroundWindow(hwnd);
            if GetForegroundWindow() == hwnd {
                break;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        send_ctrl_v();
    }
    true
}

/// 扫描码注入 Ctrl+V：虚拟键事件在只认物理键序的应用里会丢，
/// 按扫描码下发 make/break 对（与真实键盘一致，本机调试经验）
fn send_ctrl_v() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, SendInput,
    };
    let key = |scan: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: Default::default(),
                wScan: scan,
                dwFlags: if up {
                    KEYEVENTF_KEYUP | KEYEVENTF_SCANCODE
                } else {
                    KEYEVENTF_SCANCODE
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [
        key(0x1D, false), // Left Ctrl down
        key(0x2F, false), // V down
        key(0x2F, true),  // V up
        key(0x1D, true),  // Left Ctrl up
    ];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// 后台进程的前台切换兜底（drop 呼出面板专用）：SetForegroundWindow 受系统
/// 前台锁约束，zap 非前台时（drop 时刻前台是拖源应用）会被静默拒绝。注入
/// 一次 Alt 按下/抬起——系统向最近收到输入的进程授予前台指定权，键事件即
/// 刷新该归属——随后再设一次前台。失败方向 = 面板可见但无焦点，用户点一下
/// 即恢复，无损坏通路。若真机仍不凑效，备选弹药是 AttachThreadInput 与
/// 前台线程共享输入态
pub fn force_foreground(hwnd_val: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MENU, keybd_event,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    let hwnd = HWND(hwnd_val as *mut core::ffi::c_void);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        // 扫描码 0x38 = 左 Alt，与真实键盘一致（本机调试经验：注入走扫描码）
        keybd_event(VK_MENU.0 as u8, 0x38, KEYBD_EVENT_FLAGS(0), 0);
        let ok = SetForegroundWindow(hwnd).as_bool();
        keybd_event(VK_MENU.0 as u8, 0x38, KEYEVENTF_KEYUP, 0);
        ok
    }
}

/// 窗口切换源插件适配。搜索时走窗口快照枚举（Win32 调用），
/// 保持 needs_conn = false：不得在 clip 锁内执行
pub struct WinsysPlugin;

impl super::plugin::Plugin for WinsysPlugin {
    fn search(&self, query: &str, _conn: Option<&rusqlite::Connection>) -> Vec<SearchItem> {
        search(query, 5)
    }
}
