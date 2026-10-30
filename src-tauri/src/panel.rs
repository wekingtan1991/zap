//! 面板窗口编排（自 lib.rs 拆出，评审 P1-17 步 3）：呼出/隐藏、失焦即隐、
//! hold 保活、drop 落区后的开面板编排、快速粘贴直达动作、前台窗口判定系。
//! 失焦即隐的决策提纯为 should_hide_on_blur 纯函数（决策提纯纪律）——
//! 三次失焦家族事故（表单 Esc/合并键盘面/右键菜单）的同一道防线自此有单测。

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::providers;
use crate::state::{AppState, PANEL_HEIGHT, PANEL_WIDTH, WINDOW_LABEL};
use tauri::{AppHandle, Emitter, Manager};

/// 失焦即隐决策（纯函数，三场景单测锁定）：hold 安装流程保活、显示后 300ms
/// 内的焦点抖动宽限、前台仍在面板（WebView2 子窗口夺焦时 tao 会误报
/// Focused(false)，以根窗口归属为准）三者任一成立 → 不收面板
pub(crate) fn should_hide_on_blur(hold: bool, within_grace: bool, fg_in_panel: bool) -> bool {
    !hold && !within_grace && !fg_in_panel
}

/// 隐藏面板并通知前端（zap://hidden）。
/// 前端在**收起时**就清空搜索词，这样下次呼出天然是空状态——
/// 若等到 show 之后才清，用户会先看到一帧旧内容再被擦掉（表现为"闪一下"）。
pub(crate) fn hide_panel_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(WINDOW_LABEL) {
        let _ = win.hide();
    }
    // 任何隐藏路径都释放 hold：中转站视图借用 hold 对抗失焦即隐，
    // 面板已隐还挂着 hold 会变成幽灵态（下次呼出失焦不隐）
    app.state::<AppState>()
        .hold_panel
        .store(false, std::sync::atomic::Ordering::Relaxed);
    // 面板已隐 = 中转站视图不复存在，落区条恢复常态（下轮拖拽会话出条）。
    // 前端 view 状态在隐藏期间不重置，此清零与 set_stage_view 命令互补
    providers::stagedrop::clear_stage_view_flag();
    let _ = app.emit("zap://hidden", ());
}

#[tauri::command]
pub(crate) fn hide_panel(app: AppHandle) {
    hide_panel_window(&app);
}

#[tauri::command]
pub(crate) fn set_hold_panel(state: tauri::State<AppState>, hold: bool) {
    use std::sync::atomic::Ordering;
    state.hold_panel.store(hold, Ordering::Relaxed);
}

/// 前端同步中转站视图开合：视图展开期间右下角落区条不显示（视图内拖入
/// 本身就是落点，角落条冗余且与面板叠放，用户拍板 2026-10-28）。视图打开时
/// 条子若还在展示（上一轮 drop 的反馈驻留）立即收掉——放入结果由视图内的
/// 列表刷新与处理反馈呈现
#[tauri::command]
pub(crate) fn set_stage_view(app: AppHandle, active: bool) {
    providers::stagedrop::set_stage_view(&app, active);
}

/// 按开关取呼出前前台窗口（选中即粘贴的粘贴目标）；设置关闭或无记录
/// （呼出时前台就是面板/捕获失败）返回 None
pub(crate) fn take_prev_foreground(app: &AppHandle) -> Option<isize> {
    if providers::clipboard::PASTE_TO_PREV.load(std::sync::atomic::Ordering::Relaxed) {
        let state = app.state::<AppState>();
        state
            .prev_foreground
            .lock()
            .expect("prev_foreground poisoned")
            .take()
    } else {
        None
    }
}

/// 后台「取文本→回填→注入」三连：fetch 的重活（DB 全文回查等）与粘贴前的
/// 等待都在后台线程，主线程调用方只做同步存在性探查（评审 P1-07 两段式）。
/// open_item 的 clip 文本分支与 clip_merge_paste 共用；回填会被自家监听
/// 收录进历史顶部（重复内容自动去重）。retrying 退避（最坏 ~370ms）与
/// 粘贴前的等待不能占主线程消息泵
pub(crate) fn paste_bg<F>(fetch: F, prev: Option<isize>)
where
    F: FnOnce() -> Option<String> + Send + 'static,
{
    std::thread::spawn(move || {
        let Some(text) = fetch() else {
            return;
        };
        let _ = providers::clipboard::retrying(|| {
            let text = text.clone();
            arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text))
        });
        if let Some(hwnd) = prev {
            std::thread::sleep(Duration::from_millis(80));
            providers::winsys::paste_to_prev(hwnd);
        }
    });
}

/// 内容已在手的回填（合并粘贴等场景）：包一层即时 fetch 复用后台注入链路
pub(crate) fn paste_text_async(text: String, prev: Option<isize>) {
    paste_bg(move || Some(text), prev);
}

/// 呼出/隐藏面板：显示时定位到鼠标附近（多显示器内夹紧），并通知前端清空状态。
/// preset 非空时在 shown 事件后追加 zap://preset（直达热键/超级面板进保护域用），
/// 前端在 shown 重置后再设置搜索词，顺序即生效顺序
pub(crate) fn toggle_panel(app: &AppHandle, preset: Option<&str>) {
    let Some(win) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        hide_panel_window(app);
        return;
    }
    let state = app.state::<AppState>();
    // 呼出即作废旧视图残留的 hold：中转站视图借用 hold 对抗失焦即隐，
    // 视图切换路径（如中转站→设置）可能漏释放，呼出时统一清零兜底
    state.hold_panel.store(false, Ordering::Relaxed);
    // 前台窗口必须在 show 之前记：show+set_focus 后前台就换成面板了
    // （本函数与热键回调/on_window_event 的锁取值不用 expect：poison-tolerant
    // 与 stage.rs 既有写法统一，万一未来恢复 unwind 也不让 UI 交互路径成为
    // 二次 panic 点；panic="abort" 下 poison 本不可达，纯防御）
    *state
        .prev_foreground
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = capture_prev_foreground(&win);
    // 显示时定位到光标所在显示器的正中央（窗口隐藏状态下移动不触发焦点事件）
    if let Ok(pos) = app.cursor_position()
        && let Ok(Some(monitor)) = app.monitor_from_point(pos.x, pos.y)
    {
        let mp = monitor.position();
        let ms = monitor.size();
        // 面板逻辑尺寸 × 该显示器缩放比 = 物理尺寸（否则高 DPI 下会偏右下）
        let scale = monitor.scale_factor();
        let pw = PANEL_WIDTH * scale;
        let ph = PANEL_HEIGHT * scale;
        let x = mp.x as f64 + (ms.width as f64 - pw) / 2.0;
        let y = mp.y as f64 + (ms.height as f64 - ph) / 2.0;
        let _ = win.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
    }
    let _ = win.show();
    let _ = win.set_focus();
    *state
        .shown_at
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
    let _ = win.emit("zap://shown", ());
    if let Some(p) = preset {
        let _ = app.emit("zap://preset", p.to_string());
    }
    // 面板呼出后台重建应用与最近文件索引（各自 10 秒节流）：新装/卸载的软件
    // 与刚打开过的文件无需重启即可搜到
    let engine = state.engine.clone();
    drop(tauri::async_runtime::spawn_blocking(move || {
        engine.refresh_apps();
        engine.refresh_recents();
    }));
}

/// drop 落区后立即呼出中转站面板（用户拍板 2026-10-28）：show 路径整体复用
/// toggle_panel（定位/prev_foreground/hold 清零/shown/preset 全继承），差异
/// 只在「可见即 no-op 不收」——toggle 语义会把已开的工作台收掉（评审 P1），
/// 而 drop 时刻面板可见的唯一常态就是 stage 视图 hold 中（拖拽第一步焦点
/// 就交给拖源，无 hold 的视图早被失焦即隐收掉），no-op 即正确。失败与否
/// 此刻未知，不做结果分流：面板与 stage_add 并行，列表靠 zap://stage-changed
/// 刷新；失败条目不进库，面板里表现为「没有新条目」，原因仍在条内反馈（3s）
pub(crate) fn open_stage_panel(app: &AppHandle) {
    if !providers::stagedrop::STAGEDROP_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    // 自动呼出面板是可关的体验项（设置开关，默认关）：关闭时 drop 的结果
    // 反馈只落在条上（成功 1.2s / 失败 3s），面板不弹
    if !providers::stagedrop::STAGEDROP_OPEN_PANEL.load(Ordering::Relaxed) {
        return;
    }
    let Some(win) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        return;
    }
    // 本次 drop 必然呼出面板：条子即时移交（面板即反馈面）。不能等
    // set_stage_view 的 IPC 回程——实测面板先弹 ~¼s 后条子才收，共存窗
    // 即用户反馈的「视图和条子同时出现」；此刻藏条与开面板同拍发生，
    // 失败原因由 stage-drop-error 全局事件升 toast（条内反馈随条隐藏不可见）
    providers::stagedrop::hide_for_handoff(app);
    toggle_panel(app, Some("stage"));
    // 焦点兜底（评审 P1）：drop 时刻前台是拖源应用，zap 是后台进程，
    // set_focus 可能被系统前台锁拒绝——面板停在「可见但无焦点」态（没有
    // Focused(false) 转变，失焦即隐不触发，正是失焦家族的 Esc 失灵形态）。
    // 延迟核对根窗口归属，确认没拿到前台则 Alt 解锁后重设一次
    #[cfg(windows)]
    {
        let win = win.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            let Ok(hwnd) = win.hwnd() else {
                return;
            };
            if foreground_belongs_to_hwnd(hwnd) {
                return;
            }
            let ok = providers::winsys::force_foreground(hwnd.0 as isize);
            providers::diag::log(&format!(
                "stagedrop: panel focus fallback force_foreground={ok}"
            ));
        });
    }
}

/// drop 落区编排（自有 IDropTarget 回调与 wry 兜底两条路径的单一汇聚点）
pub(crate) fn handle_stage_drop(app: &AppHandle, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    // drop 即呼出中转站面板（用户拍板 2026-10-28）：压独立线程一拍——自有
    // IDropTarget 路径本就在后台线程，wry 兜底路径的调用点在主线程事件循环
    // 的拖放事件里，同步开主面板会嵌进 OLE 拖拽循环收尾；面板与入库并行，
    // 列表靠 zap://stage-changed 刷新
    {
        let app = app.clone();
        std::thread::spawn(move || open_stage_panel(&app));
    }
    let generation = providers::stagedrop::begin_processing();
    // 可见性兜底：条子被收起后 Drop 才到达时补显
    providers::stagedrop::ensure_visible(app);
    let _ = app.emit_to(
        "stagedrop",
        "zap://stagedrop-feedback",
        providers::stagedrop::PROCESSING_TEXT.to_string(),
    );
    let engine = app.state::<AppState>().engine.clone();
    let app = app.to_owned();
    drop(tauri::async_runtime::spawn_blocking(move || {
        let outcome = engine.stage_add(&paths);
        let text = providers::stagedrop::feedback_text(&outcome);
        let failed = !outcome.failed.is_empty();
        providers::stagedrop::finish_processing(generation, text.clone(), failed);
        let _ = app.emit_to("stagedrop", "zap://stagedrop-feedback", text.clone());
        // 通知主窗口：中转站视图若开着（面板 hold 期间两条放入通道并存是常态），
        // 列表需要刷新；视图未开时无人订阅，事件落空无害
        let _ = app.emit("zap://stage-changed", ());
        // 失败升全局 toast（载荷=失败原因）：开面板路径的条子已在呼出时移交
        // 隐藏（hide_for_handoff），条内 3s 失败文案不可见；不开面板路径条内
        // 反馈照常，此事件下 App 只在面板可见时呈现。成功不 toast——面板里的
        // 新条目就是反馈
        if failed {
            let _ = app.emit("zap://stage-drop-error", text);
        }
    }));
}

/// 直达热键·快速粘贴：不经面板，把最近复制的文本粘贴到当前前台窗口。
/// 剪贴板已有文本时直接注入（历史库不覆盖密码管理器来源/总开关关闭的复制，
/// 直接读库会把刚复制的内容覆盖成上一条历史）；空/非文本才回源历史库并回填。
/// 重活（剪贴板读写退避、SQLite 点查）压后台线程，不占热键回调所在的主线程
pub(crate) fn quick_paste_latest(app: &AppHandle) {
    // 焚除窗口内跳过（评审 P1-13）：密码复制后到焚除完成前，剪贴板上挂着
    // 密码明文——快速粘贴会把它原样注入前台（焦点漂移即泄给错误应用），
    // 回填覆盖也会搅乱焚除判定的序列号基准。窗口极短（默认 30s 内的直达
    // 热键按下），静默跳过落一条日志
    if providers::vault::burn_window_active() {
        providers::diag::log("quickpaste\t焚除窗口内跳过注入");
        return;
    }
    // 前台窗口在回调内先取（无阻塞）；面板自身在前台时不注入（会贴进搜索框）
    let Some(hwnd) = foreground_hwnd() else {
        return;
    };
    if foreground_in_panel(app, hwnd) {
        return;
    }
    let engine = app.state::<AppState>().engine.clone();
    std::thread::spawn(move || {
        let current = arboard::Clipboard::new()
            .ok()
            .and_then(|mut cb| cb.get_text().ok());
        let (text, needs_backfill) = match current {
            Some(t) if !t.trim().is_empty() => (t, false),
            _ => match engine.latest_clip() {
                Some(t) => (t, true),
                None => return,
            },
        };
        if needs_backfill
            && providers::clipboard::retrying(|| {
                let text = text.clone();
                arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text))
            })
            .is_err()
        {
            return;
        }
        // 稍候再注入：热键组合的修饰键可能尚未松开
        std::thread::sleep(Duration::from_millis(80));
        providers::winsys::paste_to_prev(hwnd);
    });
}

/// 前台窗口的根窗口是否属于指定 hwnd（WebView2 子窗口夺焦时 tao 会误报
/// Focused(false)，焦点判定必须落在根窗口上）
#[cfg(windows)]
pub(crate) fn foreground_belongs_to_hwnd(ours: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetForegroundWindow};
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        GetAncestor(fg, GA_ROOT) == ours
    }
}

/// 前台窗口的根窗口是否是本面板（WebView2 子窗口夺焦时 tao 会误报 Focused(false)）
#[cfg(windows)]
pub(crate) fn foreground_belongs_to_panel(window: &tauri::Window) -> bool {
    let Ok(ours) = window.hwnd() else {
        return false;
    };
    foreground_belongs_to_hwnd(ours)
}

/// 呼出前的前台窗口（剪贴板「选中即粘贴」的焦点交还目标）。前台已是面板则不记
#[cfg(windows)]
fn capture_prev_foreground(window: &tauri::WebviewWindow) -> Option<isize> {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetForegroundWindow};
    let Ok(ours) = window.hwnd() else {
        return None;
    };
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() || GetAncestor(fg, GA_ROOT) == ours {
            return None;
        }
        Some(fg.0 as isize)
    }
}

#[cfg(not(windows))]
fn capture_prev_foreground(_window: &tauri::WebviewWindow) -> Option<isize> {
    None
}

/// 前台窗口是否属于本面板（快速粘贴防自注入：面板可见时按热键，前台就是面板）
#[cfg(windows)]
fn foreground_in_panel(app: &AppHandle, fg: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor};
    let Some(win) = app.get_webview_window(WINDOW_LABEL) else {
        return false;
    };
    let Ok(ours) = win.hwnd() else {
        return false;
    };
    unsafe { GetAncestor(HWND(fg as *mut core::ffi::c_void), GA_ROOT) == ours }
}

#[cfg(not(windows))]
fn foreground_in_panel(_app: &AppHandle, _fg: isize) -> bool {
    false
}

/// 当前前台窗口句柄（快速粘贴的注入目标）
#[cfg(windows)]
fn foreground_hwnd() -> Option<isize> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe {
        let fg = GetForegroundWindow();
        (!fg.is_invalid()).then_some(fg.0 as isize)
    }
}

#[cfg(not(windows))]
fn foreground_hwnd() -> Option<isize> {
    None
}

#[cfg(test)]
mod tests {
    use super::should_hide_on_blur;

    /// 三失焦家族事故的防线决策表（评审 P1-17 步 3）：
    /// hold（安装流程）/宽限（呼出后 300ms）/前台在面板 → 任一成立不收
    #[test]
    fn 失焦即隐决策表() {
        // 三道防线各自独立生效
        assert!(
            !should_hide_on_blur(true, false, false),
            "hold 安装流程保活"
        );
        assert!(
            !should_hide_on_blur(false, true, false),
            "呼出瞬间焦点抖动宽限"
        );
        assert!(
            !should_hide_on_blur(false, false, true),
            "前台仍在面板（子窗口夺焦误报）"
        );
        // 全无防线 → 收
        assert!(
            should_hide_on_blur(false, false, false),
            "真实失焦必须收面板"
        );
        // 多防线并存同样不收（防御不互相覆盖）
        assert!(!should_hide_on_blur(true, true, true));
    }
}
