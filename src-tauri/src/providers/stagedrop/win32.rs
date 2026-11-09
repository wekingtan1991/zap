/// shell 拖拽会话判定：SysDragImage 窗口存在且可见（生命周期无文档契约，
/// 双条件兜住「常驻隐藏窗口」的形态；diag 日志采样会话起止供真机核对）
#[cfg(windows)]
fn is_shell_dragging() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsWindowVisible};
    unsafe {
        let Ok(hwnd) = FindWindowW(windows::core::w!("SysDragImage"), None) else {
            return false;
        };
        !hwnd.is_invalid() && IsWindowVisible(hwnd).as_bool()
    }
}

#[cfg(not(windows))]
fn is_shell_dragging() -> bool {
    false
}

/// 物理左键是否按住（会话保活信号）：拖拽中光标静止时系统隐藏拖拽图像
/// 窗口，但按键必然仍按着。已知局限（失效方向 = 条子提前收，降级无害）：
/// 左手用户交换鼠标键后拖的是物理右键（GetAsyncKeyState 报告交换后的逻辑
/// 键，多数换键场景仍被正确读取）、触屏拖拽无物理键、前台为提权进程
/// 时 UIPI 拦截——失效回落到纯 SysDragImage 信号（静止 1.2s 后收条）。
/// PostMessage 式注入不更新异步键态；SendInput 级注入即便读到，
/// 后果同属有界延长（条子驻留至释放），无误开通路
#[cfg(windows)]
fn left_button_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 }
}

#[cfg(not(windows))]
fn left_button_held() -> bool {
    false
}

/// 光标对当前停靠角细条矩形的邻近档（仅拖拽中每 tick 调用，节奏同 POLL_MS）。
/// 距离对细条矩形取——条就是要落的目标，出条时机 = 「目标出现在可达范围内」。
/// 任何取数失败一律 Exit：fail-safe 方向 = 不出条，与 layout_bar 的容错方向
/// 一致（取不到光标/显示器时弹条只会弹在错误位置）
#[cfg(windows)]
fn classify_proximity(app: &AppHandle) -> Near {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut pt = POINT::default();
    unsafe {
        if GetCursorPos(&mut pt).is_err() {
            return Near::Exit;
        }
    }
    let hmon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    if hmon.is_invalid() {
        return Near::Exit;
    }
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        if !GetMonitorInfoW(hmon, &mut mi).as_bool() {
            return Near::Exit;
        }
    }
    let scale = app
        .monitor_from_point(pt.x as f64, pt.y as f64)
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let w = (BAR_WIDTH * scale) as i32;
    let h = (BAR_HEIGHT * scale) as i32;
    let margin = (BAR_MARGIN_LOGICAL * scale) as i32;
    let work = WorkRect {
        left: mi.rcWork.left,
        top: mi.rcWork.top,
        right: mi.rcWork.right,
        bottom: mi.rcWork.bottom,
    };
    let (x, y) = corner_origin(position(), work, w, h, margin);
    let dist = rect_distance_px((pt.x, pt.y), (x, y, w, h));
    proximity_class(
        dist,
        (PROXIMITY_ENTER_LOGICAL * scale) as i32,
        (PROXIMITY_EXIT_LOGICAL * scale) as i32,
    )
}

#[cfg(not(windows))]
fn classify_proximity(_app: &AppHandle) -> Near {
    Near::Exit
}
