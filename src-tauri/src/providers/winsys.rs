//! 窗口切换器：枚举可见顶级窗口，搜索后置前

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
    GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

use super::SearchItem;

#[derive(Clone)]
pub struct WinEntry {
    pub hwnd: isize,
    pub title: String,
    pub process: String,
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
        let process =
            pid.and_then(super::sysutil::process_stem_of_pid)
                .unwrap_or_default();
        out.push(WinEntry {
            hwnd: hwnd.0 as isize,
            title,
            process,
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
    let entries = enumerate();
    let mut scored: Vec<(f32, &WinEntry)> = entries
        .iter()
        .filter_map(|e| {
            let hay = format!("{} {}", e.title, e.process);
            super::score_match(&hay, query).map(|s| (s, e))
        })
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
            score: score * 0.55 - i as f32,
        })
        .collect()
}

/// 把窗口拉到前台（最小化则先还原）。热键触发时系统会授予前台切换权限。
pub fn activate(hwnd_val: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };
    let hwnd = HWND(hwnd_val as *mut core::ffi::c_void);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd).as_bool()
    }
}
