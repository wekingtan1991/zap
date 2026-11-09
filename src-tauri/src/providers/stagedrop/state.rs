/// 面板隐藏路径的视图标志兜底清零（评审 P1-17 步 0）：hide_panel_window
/// 的清零与 set_stage_view 命令互补——前端 view 状态隐藏期间不重置。
/// 窄 API 替代 crate 根直写原子量（外部只经函数触碰状态）
pub fn clear_stage_view_flag() {
    STAGE_VIEW_OPEN.store(false, Ordering::Relaxed);
}

/// 前端同步中转站视图开合。打开时若条子还在（上一轮 drop 的反馈驻留），
/// 立即收掉——放入结果改由视图内的列表与处理反馈呈现；关闭只记账，
/// 下轮拖拽会话自然恢复出条
pub fn set_stage_view(app: &AppHandle, open: bool) {
    STAGE_VIEW_OPEN.store(open, Ordering::Relaxed);
    if !open {
        return;
    }
    let Some(win) = app.get_webview_window("stagedrop") else {
        return;
    };
    if !win.is_visible().unwrap_or(false) {
        return;
    }
    super::diag::log("stagedrop: hide for stage view");
    // 收条并清场（同 disable_hide：复位前端 hot + 窗口隐藏 + 状态机归零，
    // 会话若仍在进行由轮询按 STAGE_VIEW_OPEN 抑制，不会复发）
    let _ = app.emit_to("stagedrop", "zap://stagedrop-hot", false);
    let _ = win.hide();
    BAR_SHOWN.store(false, Ordering::Relaxed);
    let mut st = STATE.lock().expect("stagedrop state poisoned");
    st.session_active = false;
    st.presented = false;
    st.suppressed = false;
    st.gone_since = None;
    st.last_seen = None;
    st.feedback = None;
}

/// drop 必然呼出面板时的条子即时移交（open_stage_panel 开场调用）：面板即
/// 反馈面，条内「正在放入/已放入」与面板共存是用户不要的形态（2026-10-28
/// 实测反馈「视图和条子同时出现」——set_stage_view 的收条要等前端 view 切换
/// 的 IPC 回程，日志实测 ~¼s 共存窗）。只藏窗口与清反馈，**不清会话状态机**
/// ——session_active 保持真到轮询自然 End，防 SysDragImage 余晖触发幽灵
/// Start 重出条（此刻 STAGE_VIEW_OPEN 尚未置位，压制不住）；迟到的
/// finish_processing 结果写进已隐藏条子无害，失败原因由全局事件升 toast
pub fn hide_for_handoff(app: &AppHandle) {
    let Some(win) = app.get_webview_window("stagedrop") else {
        return;
    };
    let _ = app.emit_to("stagedrop", "zap://stagedrop-hot", false);
    let _ = win.hide();
    BAR_SHOWN.store(false, Ordering::Relaxed);
    let mut st = STATE.lock().expect("stagedrop state poisoned");
    st.feedback = None;
    // 记账「本会话已出条」：面板即反馈面，此后光标虽仍在角附近，邻近判定
    // 不得再出条（同不清会话状态机的理由——防 SysDragImage 余晖幽灵重出）
    st.presented = true;
}

/// drop 事件触达即进入处理态：条子保持展示并换处理文案，返回会话代际
/// （stage_add 完成回调凭代际写入，期间已开新会话则结果作废）
pub fn begin_processing() -> u64 {
    let mut st = STATE.lock().expect("stagedrop state poisoned");
    st.feedback = Some(Feedback {
        text: PROCESSING_TEXT.into(),
        until: Instant::now() + Duration::from_millis(PROCESSING_MS),
    });
    st.generation
}

/// stage_add 完成：换结果文案并进入反馈期（代际不匹配 = 新会话已开，作废）。
/// err = 有失败条目：失败文案（首条失败原因）停留三倍时长
pub fn finish_processing(generation: u64, text: String, err: bool) {
    let mut st = STATE.lock().expect("stagedrop state poisoned");
    if st.generation != generation {
        return;
    }
    st.feedback = Some(Feedback {
        text,
        until: Instant::now()
            + Duration::from_millis(if err { FEEDBACK_ERR_MS } else { FEEDBACK_MS }),
    });
}

/// 前端拉取当前反馈文案（挂载时同步用；None = 默认文案）
pub fn current_text() -> Option<String> {
    let st = STATE.lock().expect("stagedrop state poisoned");
    st.feedback.as_ref().map(|f| f.text.clone())
}
