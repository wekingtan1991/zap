/// 纯几何：停靠角 + 物理尺寸 + 边距 → 窗口左上角物理坐标。双态同构锚定：
/// 细条与展开卡都落在同一角原点，展开尺寸两维均大于细条 → 细条矩形天然
/// 包含在卡内，悬停光标不会被展开甩出窗口（右下角时代「向上/向左扩」的
/// 包含性论证泛化到四角，防 leave/enter 抖动循环——原专属右下角的注释语义）
fn corner_origin(corner: Corner, work: WorkRect, w: i32, h: i32, margin: i32) -> (i32, i32) {
    let x = if matches!(corner, Corner::BottomRight | Corner::TopRight) {
        work.right - w - margin
    } else {
        work.left + margin
    };
    let y = if corner.is_top() {
        work.top + margin
    } else {
        work.bottom - h - margin
    };
    (x, y)
}

/// 起点误落规避：拖拽起点（光标）落在细条矩形内时沿垂直方向让出一个条位
/// （下角上移、上角下移），仅细条态调用——展开态与细条同锚角原点且更大，
/// 光标天然在窗口内
fn avoid_drag_origin(
    corner: Corner,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    margin: i32,
    pt: (i32, i32),
) -> (i32, i32) {
    let hit = pt.0 >= x && pt.0 <= x + w && pt.1 >= y && pt.1 <= y + h;
    if !hit {
        return (x, y);
    }
    if corner.is_top() {
        (x, y + h + margin)
    } else {
        (x, y - h - margin)
    }
}

/// 光标所在显示器工作区**停靠角**定位（rcWork 扣掉任务栏；superpanel 的夹紧
/// 用整屏尺寸会压任务栏，这里不沿用）。角由设置项 stagedrop_position 决定
/// （四角枚举，默认右下）。双态布局：细条 = 会话常态；展开 = QQ 式大卡片
/// （文件悬停时），锚定见 corner_origin。拖拽起点落在细条矩形内时沿垂直向
/// 让开一个条位，防起点误落
/// pub(crate)：droptarget.rs 的 DragEnter/Leave/Drop 触发双态布局（兄弟模块访问）
#[cfg(windows)]
pub(crate) fn layout_bar(app: &AppHandle, expanded: bool) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let Some(win) = app.get_webview_window("stagedrop") else {
        return;
    };
    let (w_l, h_l) = if expanded {
        (CARD_WIDTH, CARD_HEIGHT)
    } else {
        (BAR_WIDTH, BAR_HEIGHT)
    };
    let mut pt = POINT::default();
    unsafe {
        if GetCursorPos(&mut pt).is_err() {
            return;
        }
    }
    let hmon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    if hmon.is_invalid() {
        return;
    }
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
            return;
        }
    }
    let scale = app
        .monitor_from_point(pt.x as f64, pt.y as f64)
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let w = (w_l * scale) as i32;
    let h = (h_l * scale) as i32;
    let margin = (BAR_MARGIN_LOGICAL * scale) as i32;
    let work = WorkRect {
        left: mi.rcWork.left,
        top: mi.rcWork.top,
        right: mi.rcWork.right,
        bottom: mi.rcWork.bottom,
    };
    let corner = position();
    let (mut x, mut y) = corner_origin(corner, work, w, h, margin);
    // 仅细条态做起点误落规避；展开态与细条同锚角原点，光标天然在窗口内
    if !expanded {
        (x, y) = avoid_drag_origin(corner, x, y, w, h, margin, (pt.x, pt.y));
    }
    // 尺寸与位置同用光标显示器 scale 的物理值（评审 P2-a：自定义缩放下
    // Logical 换算与位置公式取整路径不同会差 1px，物理值严格对齐）
    // 诊断：展开失效排查（2026-10-28 真机 hot 内容出现但窗口不放大，两条
    // 结果此前被 let _ = 吞掉不可见）；DragEnter 级低频调用，日志量有界
    let r_size = win.set_size(tauri::PhysicalSize::new(w, h));
    let r_pos = win.set_position(tauri::PhysicalPosition::new(x, y));
    super::diag::log(&format!(
        "stagedrop: layout corner={} expanded={expanded} {w}x{h}@{x},{y} scale={scale} \
         size_ok={} pos_ok={}",
        corner.as_str(),
        r_size.is_ok(),
        r_pos.is_ok()
    ));
}

#[cfg(not(windows))]
fn layout_bar(_app: &AppHandle, _expanded: bool) {}

fn show(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("stagedrop") {
        let _ = win.show();
        BAR_SHOWN.store(true, Ordering::Relaxed);
    }
}

fn hide(app: &AppHandle) {
    // 收条即复位前端 hot（前端评审 P2 加固：封死协议异常/源进程被杀的
    // stale-high 残留——正常路径 DragLeave/Drop 已各自复位，此为兜底）
    let _ = app.emit_to("stagedrop", "zap://stagedrop-hot", false);
    if let Some(win) = app.get_webview_window("stagedrop") {
        let _ = win.hide();
    }
    BAR_SHOWN.store(false, Ordering::Relaxed);
}

/// 出条（会话内首次或撤回后再近）：布局+显示+动画推送并记 presented。
/// 布局与 hot 依悬停镜像分流：悬停中保留 DragEnter 的展开布局并重申
/// hot=true（幂等），非悬停走细条并清 hot（顺带兜底任何失联的旧高亮，
/// 同 hide 的兜底语义）。角先于 reset 推送：前端入场动画方向依停靠角切换
/// （顶角从上滑入）；每轮出条都发而非只在设置变更时发——绕开「启动早期
/// 前端监听未就绪收不到一次性通知」的时序问题。payload 恒 start：前端只
/// 对 start 重挂重播入场动画（stale 不重挂，防拖拽中途闪动画）
fn present_bar(app: &AppHandle) {
    let hovering = HOVERING.load(Ordering::Relaxed);
    layout_bar(app, hovering);
    show(app);
    let _ = app.emit_to("stagedrop", "zap://stagedrop-hot", hovering);
    super::diag::log(&format!("stagedrop: bar presented hovering={hovering}"));
    let _ = app.emit_to("stagedrop", "zap://stagedrop-corner", position_setting());
    let _ = app.emit_to("stagedrop", "zap://stagedrop-reset", "start");
    STATE.lock().expect("stagedrop state poisoned").presented = true;
}

/// 撤条（拖离出撤条门槛，或新会话远离角时收上一轮残条）：复位前端 hot +
/// 藏窗 + 清反馈与本会话出条记账——再拖近会重新走 present_bar 全流程
fn withdraw_bar(app: &AppHandle) {
    hide(app);
    super::diag::log("stagedrop: bar withdrawn (away from corner)");
    let mut st = STATE.lock().expect("stagedrop state poisoned");
    st.presented = false;
    st.feedback = None;
}

/// 开关关闭：已显示则归位（拖拽中关闭也立即收）
fn disable_hide(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("stagedrop")
        && win.is_visible().unwrap_or(false)
    {
        let _ = app.emit_to("stagedrop", "zap://stagedrop-hot", false);
        let _ = win.hide();
        BAR_SHOWN.store(false, Ordering::Relaxed);
        let mut st = STATE.lock().expect("stagedrop state poisoned");
        st.session_active = false;
        st.gone_since = None;
        st.last_seen = None;
        st.feedback = None;
    }
}
