//! 录制热键：GetAsyncKeyState 轮询物理按键状态（不经消息/钩子投递，免疫输入法高层拦截与
//! 本进程钩子不投递的问题）。轮询粒度 15ms，对热键录制足够。前台不在面板时丢弃采集。
//! 诊断日志：%APPDATA%\com.ais.zap\capture.log

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetForegroundWindow};

static CAPTURING: AtomicBool = AtomicBool::new(false);
static PENDING: Mutex<Option<String>> = Mutex::new(None);
/// 录制最长持续 60 秒，防前端异常后轮询滞留
static STARTED: Mutex<Option<Instant>> = Mutex::new(None);

fn log(msg: &str) {
    use std::io::Write;
    if let Ok(base) = std::env::var("APPDATA") {
        let path = std::path::Path::new(&base)
            .join("com.ais.zap")
            .join("capture.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let _ = writeln!(f, "[{now:?}] {msg}");
        }
    }
}

fn key_down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) < 0 }
}

/// 与设置页约定一致的键名集：字母/数字/F1-F24/空格
fn vk_name(vk: u32) -> Option<String> {
    match vk {
        0x30..=0x39 => Some((vk as u8 as char).to_string()),
        0x41..=0x5A => Some((vk as u8 as char).to_ascii_lowercase().to_string()),
        0x20 => Some("space".into()),
        0x70..=0x87 => Some(format!("f{}", vk - 0x70 + 1)),
        _ => None,
    }
}

/// 轮询候选键（与 vk_name 一致）：数字、字母、空格、F1-F24
fn candidate_vks() -> Vec<u32> {
    let mut v = Vec::new();
    v.extend(0x30..=0x39);
    v.extend(0x41..=0x5A);
    v.push(0x20);
    v.extend(0x70..=0x87);
    v
}

pub fn start(app: tauri::AppHandle) {
    let was = CAPTURING.swap(true, Ordering::SeqCst);
    log(&format!("start invoked, already_capturing={was}"));
    if was {
        return; // 已在录制中
    }
    *STARTED.lock().expect("started poisoned") = Some(Instant::now());

    std::thread::spawn(move || {
        let our_hwnd = app
            .get_webview_window("main")
            .and_then(|w| w.hwnd().ok())
            .unwrap_or_default();
        let vks = candidate_vks();
        // 记录每个候选键的上一轮状态，只在「抬起→按下」时捕获一次（防自动重复）
        let mut prev: Vec<bool> = vks.iter().map(|&vk| key_down(vk as i32)).collect();
        while CAPTURING.load(Ordering::Relaxed) {
            let expired = STARTED
                .lock()
                .expect("started poisoned")
                .is_some_and(|t| t.elapsed() > Duration::from_secs(60));
            if expired {
                log("capture timeout (60s)");
                stop();
                break;
            }
            for (i, &vk) in vks.iter().enumerate() {
                let down = key_down(vk as i32);
                if down && !prev[i] {
                    let mut combo = String::new();
                    if key_down(0x11) {
                        combo.push_str("ctrl+");
                    }
                    if key_down(0x12) {
                        combo.push_str("alt+");
                    }
                    if key_down(0x10) {
                        combo.push_str("shift+");
                    }
                    if key_down(0x5B) || key_down(0x5C) {
                        combo.push_str("super+");
                    }
                    if !combo.is_empty() {
                        let full = format!("{combo}{}", vk_name(vk).unwrap_or_default());
                        *PENDING.lock().expect("pending poisoned") = Some(full);
                    }
                }
                prev[i] = down;
            }
            let fg = unsafe { GetForegroundWindow() };
            let root_is_ours = unsafe { GetAncestor(fg, GA_ROOT) == our_hwnd };
            if !root_is_ours {
                if let Some(c) = PENDING.lock().expect("pending poisoned").take() {
                    log(&format!("discard (not our window): {c}"));
                    // 前端不知情会一直停在「按下新组合…」录制态：丢弃时同步通知
                    let _ = app.emit("zap://hk-capture-reset", ());
                }
            } else if let Some(combo) = PENDING.lock().expect("pending poisoned").take() {
                log(&format!("captured+emit: {combo}"));
                let _ = app.emit("zap://hk-capture", combo);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        log("poller exit");
    });
}

pub fn stop() {
    log("stop invoked");
    CAPTURING.store(false, Ordering::SeqCst);
}
