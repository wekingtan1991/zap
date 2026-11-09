/// 纯判定：距离 → 邻近档
fn proximity_class(dist: i32, enter: i32, exit: i32) -> Near {
    if dist <= enter {
        Near::Enter
    } else if dist >= exit {
        Near::Exit
    } else {
        Near::Band
    }
}

/// 纯几何：点到矩形（x, y, w, h）的距离——矩形内为 0，矩形外取最近边
/// （对角走欧氏距离并向上取整；门槛判定不需要亚像素精度）
fn rect_distance_px(pt: (i32, i32), rect: (i32, i32, i32, i32)) -> i32 {
    let (px, py) = pt;
    let (rx, ry, rw, rh) = rect;
    let dx = if px < rx {
        rx - px
    } else if px > rx + rw {
        px - (rx + rw)
    } else {
        0
    };
    let dy = if py < ry {
        ry - py
    } else if py > ry + rh {
        py - (ry + rh)
    } else {
        0
    };
    ((dx * dx + dy * dy) as f64).sqrt().ceil() as i32
}

/// 单次轮询的决策结果：纯计算（无副作用），执行端只做状态落盘与窗口操作，
/// 抖动/保活/反馈/邻近交织的可测性集中在这里（见 tests::轮询决策情形覆盖）
enum Poll {
    /// 新会话开始（只记账不出条）；suppressed = 中转站视图展开中整场压制，
    /// present_now = 起点已进邻近门槛、记账后当场出条（观感同旧版即时出条）
    Start { suppressed: bool, present_now: bool },
    /// 会话中拖近停靠角：出条（会话内首次或撤回后再近）
    Present,
    /// 已出条但拖离出撤条门槛：收条还屏于拖拽本体，再拖回来重新出（迟滞防闪）
    Withdraw,
    /// 拖拽进行中且旧反馈文案已到期
    ResetStale,
    /// 信号消失但在去抖宽限内（抖动或左键保活），不动
    Hold,
    /// 宽限耗尽，会话结束
    End,
    /// 反馈/驻留未到期，保持展示
    Settle,
    /// 收条并清场
    Hide,
}

/// 决策纯函数。会话只由图像可见性开启（左键按住的框选/普通点击不开会话）：
/// - 拖拽中且无会话 → Start 记账；present_now = 未被视图压制且已进邻近
///   门槛（起点即在角附近则当场出条）
/// - 拖拽中且有会话 → 未出条且进门槛 → Present；已出条且出撤条门槛 →
///   Withdraw（迟滞带 Band 保持现状）；旧反馈到期 → ResetStale；否则 Settle
/// - 信号消失 → 去抖宽限内 Hold、耗尽 End（不变）；无会话按反馈/驻留收条
///   （不变）
///
/// near 由调用方折算（含「OLE 悬停即视为在角」的覆盖，见 spawn），本函数
/// 只认档位。stage_view_open = 中转站视图展开中：新会话照常记账（递增代际
/// 作废旧反馈）但整场压制出条——视图内拖入即落点，角落条冗余；会话中途
/// 关视图也不补出，下轮会话恢复。drag_out_active = 拖拽出会话进行中：防御
/// 门——默认光标拖出不产 SysDragImage（PoC 2 实证），但一旦上游/主题行为
/// 变化产生拖影窗口，拖出会被误判为拖入会话弹条并把临时文件误收进中转站
/// （R01-P1）；无会话时一律 Hold 不开新会话
fn decide(
    st: &DropState,
    dragging: bool,
    stage_view_open: bool,
    drag_out_active: bool,
    near: Near,
    now: Instant,
) -> Poll {
    if drag_out_active && !st.session_active {
        return Poll::Hold;
    }
    if dragging {
        if !st.session_active {
            Poll::Start {
                suppressed: stage_view_open,
                present_now: !stage_view_open && near == Near::Enter,
            }
        } else if !st.presented && !st.suppressed && near == Near::Enter {
            Poll::Present
        } else if st.presented && near == Near::Exit {
            Poll::Withdraw
        } else if st.feedback.as_ref().is_some_and(|f| now >= f.until) {
            Poll::ResetStale
        } else {
            Poll::Settle
        }
    } else if st.session_active {
        if st
            .last_seen
            .is_some_and(|t| now.duration_since(t) < Duration::from_millis(GONE_DEBOUNCE_MS))
        {
            Poll::Hold
        } else {
            Poll::End
        }
    } else {
        let expired = st.feedback.as_ref().is_none_or(|f| now >= f.until);
        if !expired {
            return Poll::Settle;
        }
        let dwell_ok = st.feedback.is_some()
            || st
                .gone_since
                .is_some_and(|t| now.duration_since(t) >= Duration::from_millis(HIDE_DWELL_MS));
        if dwell_ok { Poll::Hide } else { Poll::Settle }
    }
}
