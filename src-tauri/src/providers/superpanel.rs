//! 超级面板：全局热键触发，光标旁弹出动作菜单。
//! 触发方式 = 直达热键（与账号库/快速粘贴同机制，零冲突）——按下时抓取
//! 选中文本（模拟 Ctrl+C，抓取前记录剪贴板现场 + 入库抑制窗，抓取后立即
//! 恢复原内容），按内容识别生成动作菜单，在光标旁的 superpanel 窗口展示。
//! 早版用 WH_MOUSE_LL 长按右键手势，因与各应用右键菜单冲突风险大已退役

use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

/// Ctrl+C 后等待剪贴板变化的上限（慢应用放宽到 400ms）
const CAPTURE_TIMEOUT_MS: u64 = 400;

/// 动作菜单项（id 供 superpanel_act 执行，label 直接渲染）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuperAction {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuperActionsPayload {
    pub text: String,
    pub actions: Vec<SuperAction>,
}

/// 最近一次菜单内容：前端聚焦窗口时主动拉取（superpanel_payload 命令）。
/// 事件推送之外的双保险——WebView2 隐藏窗口的事件投递时序不可全信，
/// 而 show 后 set_focus 必触发前端 focus 事件，拉取路径零时序依赖
pub static LAST_PAYLOAD: std::sync::Mutex<Option<SuperActionsPayload>> =
    std::sync::Mutex::new(None);

/// 「在 zap 中打开」动作回调（评审 P1-17 步 0）：providers 不反向依赖
/// crate 根的 toggle_panel，lib.rs setup 时经 register_open_panel 注入
type OpenPanel = std::sync::Arc<dyn Fn(&tauri::AppHandle, String) + Send + Sync>;
static ON_OPEN_PANEL: std::sync::OnceLock<OpenPanel> = std::sync::OnceLock::new();

pub fn register_open_panel(f: OpenPanel) {
    let _ = ON_OPEN_PANEL.set(f);
}

/// 抓取结束后的处置（纯函数，真值表单测锁定）：seq 未变 = Ctrl+C 没有生效、
/// 剪贴板未被改动，静默放弃且无需恢复；seq 变了 = 用户原剪贴板已被抓取结果
/// 覆盖，**所有出口都必须恢复现场**——包括复制到的不是文本（文件选区）与
/// 空文本两个旧实现直接 return 漏恢复的分支
enum CaptureOutcome {
    /// 显示动作菜单（text 已 trim 非空）
    Show(String),
    /// 只恢复现场，不显示
    RestoreOnly,
    /// 剪贴板未被改动，什么都不做
    Noop,
}

fn capture_outcome(seq_changed: bool, captured: Option<String>) -> CaptureOutcome {
    if !seq_changed {
        return CaptureOutcome::Noop;
    }
    match captured {
        Some(t) if !t.trim().is_empty() => CaptureOutcome::Show(t.trim().to_string()),
        // 复制到非文本（arboard get_text 失败）或空文本：现场已覆盖，仍要恢复
        _ => CaptureOutcome::RestoreOnly,
    }
}

/// 恢复抓取前的剪贴板现场。文本走 set_text + retrying（剪贴板被占用可瞬时
/// 失败）；图片走 set_image_dib 直写——arboard set_image 要先编码整张 PNG，
/// 大图秒级（ccf7ac7 实测 39MB 5.8s），恢复路径同样不能踩
fn restore_clipboard(orig_text: Option<String>, orig_img: Option<arboard::ImageData>) {
    if let Some(t) = orig_text {
        let _ = super::clipboard::retrying(move || {
            let t = t.clone();
            arboard::Clipboard::new().and_then(|mut cb| cb.set_text(t))
        });
    } else if let Some(img) = orig_img {
        // arboard ImageData 的 width/height 本就是 usize，直接传
        let _ = super::clipboard::set_image_dib(img.width, img.height, &img.bytes);
    }
}

/// 前端拉取当前菜单内容
pub fn current_payload() -> Option<SuperActionsPayload> {
    LAST_PAYLOAD.lock().expect("super payload poisoned").clone()
}

/// 触发入口（直达热键按下时调用）：主面板可见时让位；
/// 抓取窗口期抑制入库，选中文本 → 识别生成动作 → 光标旁展示
pub fn trigger_from_hotkey(app: &AppHandle) {
    // 主面板可见时不打扰（超级面板与主面板不同时出现）
    if app
        .get_webview_window("main")
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false)
    {
        return;
    }
    #[cfg(windows)]
    let t0 = Instant::now();
    #[cfg(windows)]
    {
        let Some((x, y)) = cursor_pos() else {
            return;
        };
        // 抓取窗口期抑制入库：注入的 Ctrl+C 与恢复现场都不进剪贴板历史
        super::clipboard::begin_self_copy();
        use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
        let seq0 = unsafe { GetClipboardSequenceNumber() };
        let orig_text = arboard::Clipboard::new()
            .and_then(|mut cb| cb.get_text())
            .ok();
        // 原剪贴板非文本时（截图等）读图恢复
        let orig_img = if orig_text.is_none() {
            arboard::Clipboard::new()
                .and_then(|mut cb| cb.get_image())
                .ok()
        } else {
            None
        };
        inject_ctrl_c();
        // 轮询序列号等待复制生效
        let deadline = Instant::now() + Duration::from_millis(CAPTURE_TIMEOUT_MS);
        loop {
            if unsafe { GetClipboardSequenceNumber() } != seq0 {
                break;
            }
            if Instant::now() >= deadline {
                return; // 无选中内容/应用拦了 Ctrl+C：seq 未变剪贴板未动，静默放弃
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let seq_changed = unsafe { GetClipboardSequenceNumber() } != seq0;
        let captured = arboard::Clipboard::new()
            .and_then(|mut cb| cb.get_text())
            .ok();
        // 处置按真值表分流：恢复现场是所有「seq 变了」出口的必经步骤，
        // 旧的散装实现漏掉了非文本与空文本两个出口（capture_outcome 锁契约）
        match capture_outcome(seq_changed, captured) {
            CaptureOutcome::Noop => {}
            CaptureOutcome::RestoreOnly => restore_clipboard(orig_text, orig_img),
            CaptureOutcome::Show(text) => {
                restore_clipboard(orig_text, orig_img);
                super::diag::log(&format!(
                    "super\t抓取+识别耗时\t{}ms\t文本 {} 字符",
                    t0.elapsed().as_millis(),
                    text.chars().count()
                ));
                show_panel(app, x, y, text);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// 当前光标物理坐标（热键触发时按下点即选中文本旁）
#[cfg(windows)]
fn cursor_pos() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT::default();
    unsafe { GetCursorPos(&mut pt).ok()? };
    Some((pt.x, pt.y))
}

/// 扫描码注入 Ctrl+C（同 paste 的 make/break 成对下发）
#[cfg(windows)]
fn inject_ctrl_c() {
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
    // Ctrl(0x1D)↓ C(0x2E)↓ C↑ Ctrl↑
    let inputs = [
        key(0x1D, false),
        key(0x2E, false),
        key(0x2E, true),
        key(0x1D, true),
    ];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// 超级面板输入卫兵：MB 级选文全文 serde_json 解析可把动作生成卡住数百毫秒
/// （热键后台线程，但恢复现场在此之后——恢复越晚用户剪贴板错位越久）。
/// 直接复用 jsontools 的 256KB 上限，同链路口径一致
use super::jsontools::MAX_ANALYZE_BYTES as MAX_INPUT_BYTES;

/// 按内容识别生成动作：特定识别在前（带答案），通用动作兜底，上限 5 条
pub fn build_actions(text: &str) -> Vec<SuperAction> {
    let t = text.trim();
    let mut actions: Vec<SuperAction> = Vec::new();
    // 文本工具识别（停用时 convert_all 报错 → 跳过特定动作）；超卫兵跳过
    if t.len() <= MAX_INPUT_BYTES
        && let Ok(bundle) = super::texttools::convert_all(t)
    {
        if let Some(answer) = bundle.ts_to_date {
            actions.push(SuperAction {
                id: "ts_answer".into(),
                label: format!("复制换算结果（{answer}）"),
            });
        } else if let Some(dec) = &bundle.base64_dec {
            let preview: String = dec.chars().take(18).collect();
            actions.push(SuperAction {
                id: "dec_answer".into(),
                label: format!("复制解码结果（{preview}…）"),
            });
        } else if let Some(dec) = &bundle.url_dec {
            let preview: String = dec.chars().take(18).collect();
            actions.push(SuperAction {
                id: "dec_answer".into(),
                label: format!("复制解码结果（{preview}…）"),
            });
        }
    }
    if t.len() <= MAX_INPUT_BYTES && serde_json::from_str::<serde_json::Value>(t).is_ok() {
        actions.push(SuperAction {
            id: "json_pretty".into(),
            label: "格式化 JSON 并复制".into(),
        });
        actions.push(SuperAction {
            id: "json_compact".into(),
            label: "压缩 JSON 并复制".into(),
        });
    }
    actions.push(SuperAction {
        id: "copy".into(),
        label: "复制内容".into(),
    });
    actions.push(SuperAction {
        id: "open_panel".into(),
        label: "在 zap 中打开".into(),
    });
    actions.push(SuperAction {
        id: "web".into(),
        label: "网页搜索".into(),
    });
    // 上限 6：当前动作组合实际最多 5 条（识别答案与 JSON 识别互斥），
    // 留 1 条余量给未来新增动作，防「网页搜索」这类兜底被静默裁掉
    actions.truncate(6);
    actions
}

/// 光标旁展示：夹进光标所在显示器，尺寸随动作条数
#[cfg(windows)]
fn show_panel(app: &AppHandle, x: i32, y: i32, text: String) {
    let Some(win) = app.get_webview_window("superpanel") else {
        return;
    };
    let actions = build_actions(&text);
    // 窗口高度随条数：每条 40px + 上下内边距（与前端样式对齐）
    let h = 16.0 + actions.len() as f64 * 40.0 + 12.0;
    let _ = win.set_size(tauri::LogicalSize::new(264.0, h));
    let mut px = x as f64 + 12.0;
    let mut py = y as f64 + 12.0;
    if let Ok(pos) = app.cursor_position()
        && let Ok(Some(monitor)) = app.monitor_from_point(pos.x, pos.y)
    {
        let mp = monitor.position();
        let ms = monitor.size();
        // 光标坐标是物理像素；窗口尺寸按缩放比换算成物理值夹紧
        let scale = monitor.scale_factor();
        let pw = 264.0 * scale;
        let ph = h * scale;
        px = (px.min(mp.x as f64 + ms.width as f64 - pw - 4.0)).max(mp.x as f64 + 4.0);
        py = (py.min(mp.y as f64 + ms.height as f64 - ph - 4.0)).max(mp.y as f64 + 4.0);
    }
    let _ = win.set_position(tauri::PhysicalPosition::new(px as i32, py as i32));
    let payload = SuperActionsPayload { text, actions };
    *LAST_PAYLOAD.lock().expect("super payload poisoned") = Some(payload.clone());
    // 推送 + 前端 focus 拉取双通道（拉取是兜底主路径，见 LAST_PAYLOAD 注释）
    let _ = win.emit("zap://super-actions", &payload);
    let _ = win.show();
    let _ = win.set_focus();
}

#[cfg(not(windows))]
fn show_panel(app: &AppHandle, _x: i32, _y: i32, text: String) {
    let _ = (app, text);
}

/// 执行动作（superpanel_act 命令；执行后前端负责隐藏窗口）
pub fn run_action(app: &AppHandle, action: &str, text: &str) -> Result<(), String> {
    let copy = |content: String| -> Result<(), String> {
        super::clipboard::begin_self_copy();
        // 抓取内容可能含敏感信息（来自任意前台应用的选中文本）：与 vault
        // 域复制统一走受保护通道，不进 Win+V 与云剪贴板（评审 P1-13）
        super::clipboard::copy_text_private(&content)
    };
    match action {
        "copy" => copy(text.to_string()),
        // 换算/解码结果复制：以进入菜单时的文本重新转换（识别器幂等）
        "ts_answer" | "dec_answer" => {
            let bundle = super::texttools::convert_all(text.trim())?;
            let answer = bundle
                .ts_to_date
                .or(bundle.base64_dec)
                .or(bundle.url_dec)
                .ok_or("内容已无法换算")?;
            copy(answer)
        }
        "json_pretty" => copy(super::jsontools::format_text(text, true)?),
        "json_compact" => copy(super::jsontools::format_text(text, false)?),
        "open_panel" => {
            superpanel_hide(app);
            // 文本作为搜索词进主面板：probe 识别管线会自动给出识别条目。
            // 开面板经注册回调上抛（评审 P1-17 步 0）：providers 不再反向
            // 依赖 crate 根的 toggle_panel——lib.rs setup 时注入
            if let Some(f) = ON_OPEN_PANEL.get() {
                f(app, text.to_string());
            }
            Ok(())
        }
        "web" => {
            use tauri_plugin_opener::OpenerExt;
            let url = format!(
                "https://www.bing.com/search?q={}",
                urlencoding::encode(text.trim())
            );
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|e| e.to_string())
        }
        _ => Err("未知动作".into()),
    }
}

/// 收起超级面板窗口
pub fn superpanel_hide(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("superpanel") {
        let _ = win.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 抓取处置真值表() {
        use CaptureOutcome::*;
        // seq 未变：无论抓没抓到，剪贴板都没被动过——静默放弃且不恢复
        assert!(matches!(capture_outcome(false, None), Noop));
        assert!(matches!(capture_outcome(false, Some("x".into())), Noop));
        // seq 变 + 抓到非空文本：显示（trim 后）
        assert!(matches!(capture_outcome(true, Some("  hi \n".into())), Show(t) if t == "hi"));
        // seq 变 + 抓到空文本：现场已被覆盖，只恢复不显示（旧实现漏恢复）
        assert!(matches!(
            capture_outcome(true, Some("   ".into())),
            RestoreOnly
        ));
        // seq 变 + 非文本（get_text 失败）：文件选区场景，仍要恢复（旧实现漏恢复）
        assert!(matches!(capture_outcome(true, None), RestoreOnly));
    }

    #[test]
    fn 超卫兵内容不产生json与换算动作() {
        let big = format!("\"{}\"", "x".repeat(MAX_INPUT_BYTES + 1));
        let actions = build_actions(&big);
        assert!(
            !actions.iter().any(|a| a.id.starts_with("json_")),
            "超卫兵不跑 JSON 全文解析"
        );
        // 普通动作仍在（兜底不被裁掉）
        assert!(actions.iter().any(|a| a.id == "copy"));
    }
}
