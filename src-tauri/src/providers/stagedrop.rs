//! 全局拖拽落区条：任何应用开始 shell 拖拽（Explorer/桌面拖文件）时，光标
//! 所在显示器工作区的停靠角（设置项 stagedrop_position，四角可选默认右下）
//! 弹出「拖到这，放入中转站」细条，拖入松手即入中转站。
//!
//! 检测原理：shell 拖拽会话期间 shell32 会创建类名 `SysDragImage` 的拖拽图像
//! 窗口（无文档契约，IsWindowVisible 双条件判定；diag 日志记录会话起止，供
//! 真机核对该窗口的实际生命周期）。轮询是唯一免注入的通用手段——WH_CBT 全局
//! 钩子需 DLL 注入，EVENT_SYSTEM_DRAGDROPSTART 实测不可靠；升级路径 =
//! SetWinEventHook。
//!
//! **信号抖动（真机实测，zap.log 32 秒采样 36 次起止）**：SysDragImage 在拖拽
//! 会话内会随光标静止/悬停目标反复短暂消失（gap 0.1~2s），会话判定必须做
//! 1.2s 去抖宽限，否则条子反复显隐+重定位（用户视角 = 躲猫猫）。
//!
//! **drop 接收走自有 IDropTarget**（本文件 StageDropTarget）：wry 给 WebView2
//! 设的 AllowExternalDrop=false 路径在该窗口拒绝外部拖放（真机实测光标恒为
//! 禁止、Drop 永不达），故在 WebView2 子窗口上撤销 wry 注册、挂自有目标——
//! 效果恒 COPY，悬停/离开发 zap://stagedrop-hot 驱动前端高亮，Drop 直取
//! CF_HDROP 进统一处理（lib.rs handle_stage_drop）。wry→on_window_event
//! 路径保留为兜底（注册失败时仍可用）。
//!
//! 生命周期为 Rust 单写者状态机（评审 P0 修订）：反馈用「挂起到期自恢复」
//! 而非前端通知解除；会话代际号防「反馈期内开始新一轮拖拽」的交织。落区
//! 窗口绝不抢焦点——`focusable:false` 才产生 WS_EX_NOACTIVATE。
//!
//! **已知未解之谜（wry/WebView2 升级复核点）**：main 面板与本窗口同为 wry
//! webview（同为 AllowExternalDrop(false) + wry 自有 IDropTarget 注册，见
//! wry webview2/mod.rs 的 drag_drop_controller），但视图内拖入在 main 实测
//! 可用、在本窗口恒被拒——两窗口可识别的差异只有 focusable:false 与
//! shadow:false，均未证实为根因。升级任一依赖后两条路径的行为都可能漂移，
//! 需真机复测两个窗口的拖放。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

/// 运行时开关：启动从 appconfig 加载、设置保存时同步
pub static STAGEDROP_ENABLED: AtomicBool = AtomicBool::new(true);

/// 拖入松手后自动呼出中转站面板（设置开关，默认关）：open_stage_panel 的
/// 门控，关闭时放入结果仍由条内反馈呈现（成功 1.2s / 失败 3s）。启动从
/// appconfig 加载、设置保存时同步
pub static STAGEDROP_OPEN_PANEL: AtomicBool = AtomicBool::new(false);

/// 落区条停靠角（设置项 stagedrop_position）：相对拖拽发生时**光标所在**
/// 显示器的工作区。字符串值与存档/配置文件/前端 Select 共用
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Corner {
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
}

impl Corner {
    /// 存档/线协议字符串 → 枚举（非法值 None，调用方保持原值不落库）
    pub fn parse(s: &str) -> Option<Corner> {
        match s {
            "bottom_right" => Some(Corner::BottomRight),
            "bottom_left" => Some(Corner::BottomLeft),
            "top_right" => Some(Corner::TopRight),
            "top_left" => Some(Corner::TopLeft),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Corner::BottomRight => "bottom_right",
            Corner::BottomLeft => "bottom_left",
            Corner::TopRight => "top_right",
            Corner::TopLeft => "top_left",
        }
    }

    /// 垂直半区语义：起点误落规避的让位方向依此分流
    fn is_top(self) -> bool {
        matches!(self, Corner::TopRight | Corner::TopLeft)
    }
}

/// 当前停靠角（u8 = 枚举声明序，读回映射见 position）：启动从 appconfig
/// 加载、设置保存时同步。layout_bar 在会话起点/悬停展开/落点收条时都读，
/// 频度高但写者只有设置路径，无锁原子足够
static POSITION: AtomicU8 = AtomicU8::new(0);

fn position() -> Corner {
    match POSITION.load(Ordering::Relaxed) {
        1 => Corner::BottomLeft,
        2 => Corner::TopRight,
        3 => Corner::TopLeft,
        _ => Corner::BottomRight,
    }
}

/// 设置同步入口：非法值静默忽略（坏存档/脏数据不落角，保持当前值）
pub fn set_position_setting(s: &str) {
    if let Some(c) = Corner::parse(s) {
        POSITION.store(c as u8, Ordering::Relaxed);
    }
}

/// 当前角设置值（DTO 下发/配置导出用）
pub fn position_setting() -> &'static str {
    position().as_str()
}

/// 停靠角变更后立即可见化：条子正显示时按新角重排（保持当前悬停态）；
/// 隐藏态不动——下轮会话起点的 layout_bar 自然用新角
pub fn relayout_if_shown(app: &AppHandle) {
    if BAR_SHOWN.load(Ordering::Relaxed) {
        layout_bar(app, HOVERING.load(Ordering::Relaxed));
    }
}

/// 条子当前是否应处于展示态（show/hide/ensure_visible/disable_hide 各写入点
/// 维护）。开关关闭的轮询分支据此短路：is_visible 是跨线程投递主线程的查询，
/// 每 80ms 空转一次纯属浪费；BAR_SHOWN 恒真才进一次真实核对
static BAR_SHOWN: AtomicBool = AtomicBool::new(false);

/// 轮询间隔（FindWindowW 微秒级，恒定节奏；评审否决了 GetAsyncKeyState 分档：
/// 物理键/UIPI/锁屏/触屏四坑，收益无感）
const POLL_MS: u64 = 80;
/// 会话判定去抖：SysDragImage 消失持续超过该时长才认定拖拽结束（真机实测
/// 会话内抖动 gap 0.1~2s，短于此前 300ms 驻留即躲猫猫）
const GONE_DEBOUNCE_MS: u64 = 1200;
/// 会话结束后的最短驻留：等 Drop 事件链路走完
const HIDE_DWELL_MS: u64 = 200;
/// drop 结果反馈的展示时长（失败态分档加长：失败原因常是长路径，
/// 1.2s 读不完就闪没，用户失去重试线索）
const FEEDBACK_MS: u64 = 1200;
const FEEDBACK_ERR_MS: u64 = 3000;
/// stage_add 处理态的自愈上限（网络路径 metadata 可达秒级；超时轮询照常隐藏）
const PROCESSING_MS: u64 = 4000;

/// 落区条逻辑尺寸（须与 tauri.conf.json 的 stagedrop 窗口一致）。高度 44 =
/// 去扁平（2026-10-28）：CSS 外壳上下各 6px 透明边距后卡片本体 32px，36 的
/// 时代卡片仅 24px、装下 16px 图标+12.5px 文本后垂直富余约 2px/边，观感扁平。
/// 垂直呼吸感唯一来自窗口高度预算——卡片是 height:100% + border-box，只改
/// CSS 垂直 padding 会被窗口高度裁掉，等于没改
const BAR_WIDTH: f64 = 280.0;
const BAR_HEIGHT: f64 = 44.0;
/// 悬停展开的大卡片尺寸（QQ 式效果）：与细条同锚停靠角原点（见
/// corner_origin）——展开尺寸两维均大于细条，原细条区域必然落在卡片内部，
/// 悬停光标不会因展开被甩出窗口（否则 leave/enter 抖动循环）
const CARD_WIDTH: f64 = 300.0;
const CARD_HEIGHT: f64 = 264.0;

pub const PROCESSING_TEXT: &str = "正在放入中转站…";

/// 反馈态：反馈期内轮询不得隐藏条子，到期自愈
struct Feedback {
    text: String,
    until: Instant,
}

/// 单写者状态：轮询线程是唯一的显隐执行者；drop 处理只改反馈字段
struct DropState {
    /// 当前是否认定处于 shell 拖拽会话（去抖后的判定）
    session_active: bool,
    /// 最近一次检测到 SysDragImage 的时刻（去抖计时的锚点）
    last_seen: Option<Instant>,
    /// 会话结束时刻（无反馈时的最短驻留计时）
    gone_since: Option<Instant>,
    feedback: Option<Feedback>,
    /// 会话代际：新会话递增，过期代际的 stage_add 结果作废
    generation: u64,
}

static STATE: Mutex<DropState> = Mutex::new(DropState {
    session_active: false,
    last_seen: None,
    gone_since: None,
    feedback: None,
    generation: 0,
});

/// OLE 悬停镜像（DragEnter 带文件置真、DragLeave/Drop 复位；OLE 回调在主线程
/// 无竞争）。轮询线程会话起点据此分流布局——悬停中说明 DragEnter 已把窗口
/// 展开成大卡片并置前端 hot，Start 若照旧收条布局，窗口缩回细条而前端 hot
/// 仍真 =「细条上溢出大图标、标题被裁」的错乱态（真机 2026-10-28：上一轮
/// 反馈驻留期内立刻再拖、光标已在条矩形内时 DragEnter 先于本轮 Start 命中）
static HOVERING: AtomicBool = AtomicBool::new(false);

/// 中转站视图是否展开（前端 set_stage_view 同步、面板隐藏路径统一清零）。
/// 视图开着时右下角落区条不再显示：视图内拖入本身就是落点（自有
/// 「松手放入中转站」提示），角落条冗余且与面板叠放（用户拍板 2026-10-28）
pub static STAGE_VIEW_OPEN: AtomicBool = AtomicBool::new(false);

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
    st.gone_since = None;
    st.last_seen = None;
    st.feedback = None;
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

/// 单次轮询的决策结果：纯计算（无副作用），执行端只做状态落盘与窗口操作，
/// 抖动/保活/反馈交织的可测性集中在这里（见 tests::轮询决策情形覆盖）
enum Poll {
    /// 新会话开始；suppressed = 中转站视图展开中，只记账不出条
    Start { suppressed: bool },
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

/// 决策与原 if-else 链逐分支等价：dragging 分两种（新会话/进行中），
/// 无信号且会话中分两种（宽限内/耗尽），无会话分三种（反馈未到期/到期但
/// 驻留不足/到期且可收——反馈存在本身就是「到期即收」的充分条件）。
/// stage_view_open = 中转站视图展开中：新会话照常记账（递增代际作废旧
/// 反馈）但不出条——视图内拖入即落点，角落条冗余；会话中途关视图也不
/// 补出，下轮会话恢复（b523d22 纪律：可测性集中在 decide，见 tests）
fn decide(st: &DropState, dragging: bool, stage_view_open: bool, now: Instant) -> Poll {
    if dragging {
        if !st.session_active {
            Poll::Start {
                suppressed: stage_view_open,
            }
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

/// drop 接收时的可见性兜底：条子被收起后 Drop 才到达时补显（幂等，细条态）
pub fn ensure_visible(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("stagedrop")
        && !win.is_visible().unwrap_or(false)
    {
        layout_bar(app, false);
        let _ = win.show();
        BAR_SHOWN.store(true, Ordering::Relaxed);
    }
}

/// stage_add 结果 → 展示文案（added = 真实新增数，纯重复显示「已在」更准确）
pub fn feedback_text(outcome: &super::stage::StageAddOutcome) -> String {
    if outcome.failed.is_empty() {
        if outcome.added > 0 {
            format!("已放入 {} 条 · 共 {} 条", outcome.added, outcome.total)
        } else {
            format!("已在中转站（已置顶）· 共 {} 条", outcome.total)
        }
    } else if outcome.added > 0 {
        format!(
            "已放入 {} 条，{} 条失败",
            outcome.added,
            outcome.failed.len()
        )
    } else {
        outcome.failed[0].clone()
    }
}

/// 检测线程：唯一有权显隐条子的执行者（评审 P0：不依赖前端通知/解除）。
/// 启动即无条件 spawn——线程内按 STAGEDROP_ENABLED 门控（关闭态仅每 tick 一次
/// is_visible 探测），运行时重开开关立即生效，无需重启
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(POLL_MS));
            if !STAGEDROP_ENABLED.load(Ordering::Relaxed) {
                if BAR_SHOWN.load(Ordering::Relaxed) {
                    disable_hide(&app);
                }
                continue;
            }
            let dragging = is_shell_dragging();
            let now = Instant::now();
            let mut st = STATE.lock().expect("stagedrop state poisoned");
            if dragging {
                st.last_seen = Some(now);
            } else if st.session_active && left_button_held() {
                // 会话保活（用户实测：光标静止时系统持续隐藏拖拽图像窗口，
                // SysDragImage 消失 ≠ 拖拽结束）——物理左键仍按住即仍在拖，
                // 刷新锚点。图像可见性驱动会话出现，按键状态驱动会话存续
                st.last_seen = Some(now);
            }

            match decide(&st, dragging, STAGE_VIEW_OPEN.load(Ordering::Relaxed), now) {
                Poll::Start { suppressed } => {
                    // 新会话：代际递增作废旧反馈、按当前光标重新定位。只由图像
                    // 可见性触发——左键按住（框选/普通点击）不得误开。
                    // suppressed = 中转站视图展开中：会话记账照做，角落条不出
                    // （视图在会话中途关闭也不补出条——下轮会话恢复）
                    let hovering = HOVERING.load(Ordering::Relaxed);
                    st.session_active = true;
                    st.generation += 1;
                    st.feedback = None;
                    st.gone_since = None;
                    drop(st);
                    if suppressed {
                        super::diag::log("stagedrop: drag session start suppressed (stage view)");
                        continue;
                    }
                    // 布局与 hot 双双依悬停镜像分流：悬停中保留 DragEnter 的
                    // 展开布局并重申 hot=true（幂等），非悬停才走细条并清 hot
                    // （顺带兜底任何失联的旧高亮，同 hide 的兜底语义）
                    layout_bar(&app, hovering);
                    show(&app);
                    let _ = app.emit_to("stagedrop", "zap://stagedrop-hot", hovering);
                    super::diag::log(&format!(
                        "stagedrop: drag session start hovering={hovering}"
                    ));
                    // 角先于 reset 推送：前端入场动画方向依停靠角切换（顶角
                    // 从上滑入）。每轮会话起点都发而非只在设置变更时发——绕开
                    // 「启动早期前端监听未就绪收不到一次性通知」的时序问题
                    let _ = app.emit_to("stagedrop", "zap://stagedrop-corner", position_setting());
                    // payload 分流：前端只对 start 重挂重播入场动画——
                    // stale 若也重挂，长拖拽中反馈到期会把滑入动画重播在拖拽中途
                    let _ = app.emit_to("stagedrop", "zap://stagedrop-reset", "start");
                }
                Poll::ResetStale => {
                    // 拖拽进行中：旧反馈文案到期复位（长拖拽不残留上一轮结果）。
                    // 只复位文案，不动会话动画（见上）
                    st.feedback = None;
                    drop(st);
                    let _ = app.emit_to("stagedrop", "zap://stagedrop-reset", "stale");
                }
                Poll::Hold | Poll::Settle => {
                    // 宽限期内视作仍在拖 / 反馈驻留未到期：不动条子
                    drop(st);
                }
                Poll::End => {
                    st.session_active = false;
                    st.gone_since = Some(now);
                    super::diag::log("stagedrop: drag session end");
                    drop(st);
                }
                Poll::Hide => {
                    // 反馈到期或驻留过期：清场收条
                    st.feedback = None;
                    st.gone_since = None;
                    st.last_seen = None;
                    drop(st);
                    hide(&app);
                }
            }
        }
    });
}

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

/// 工作区物理矩形（layout_bar 从 MONITORINFO.rcWork 搬运；独立结构使几何
/// 纯函数不依赖 windows 类型，测试可在任意平台跑）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorkRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

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
#[cfg(windows)]
fn layout_bar(app: &AppHandle, expanded: bool) {
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
    let margin = (12.0 * scale) as i32;
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

// ---------- 自有 IDropTarget（drop 接收主路径，见模块头） ----------

#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM};
#[cfg(windows)]
use windows::Win32::System::Ole::IDropTarget;

/// 在 stagedrop 的 WebView2 子窗口上注册自有 IDropTarget（替换 wry 的注册）。
/// WebView2 初始化异步、子窗口可能晚于 setup 就绪：后台线程经 run_on_main_thread
/// 尝试，先 300ms×10 快速阶段（就绪等待），耗尽后转 60s 低频常驻重试——wry
/// 兜底路径实测收不到 drop，慢机器上一次性耗尽即永久静默失效不可接受。
/// 注册线程 = 窗口所属主线程（OLE STA 要求），故必须经 run_on_main_thread。
#[cfg(windows)]
pub fn register_drop_target(app: &AppHandle) {
    const PENDING: usize = usize::MAX; // 哨兵：闭包未完成（n=0 是合法完成值）
    const FAST_ATTEMPTS: u32 = 10;
    let app = app.clone();
    std::thread::spawn(move || {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            // run_on_main_thread 的闭包要求 'static，经 Arc 回传结果
            let done = std::sync::Arc::new(AtomicUsize::new(PENDING));
            {
                let done_cb = std::sync::Arc::clone(&done);
                let a = app.clone();
                if app
                    .run_on_main_thread(move || {
                        let n = unsafe { try_register_children(&a) };
                        done_cb.store(n, Ordering::Relaxed);
                    })
                    .is_err()
                {
                    return; // 应用退出中
                }
                // 等待闭包完成（哨兵翻掉即完成，n=0 也算——交由下方重试逻辑处理）
                while done.load(Ordering::Relaxed) == PENDING {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            let n = done.load(Ordering::Relaxed);
            if n > 0 {
                super::diag::log(&format!(
                    "stagedrop: own drop target registered on {n} child hwnd(s) (attempt {attempt})"
                ));
                return;
            }
            if attempt == FAST_ATTEMPTS {
                super::diag::log(
                    "stagedrop: drop target registration failed 10 fast attempts; retrying every 60s",
                );
            }
            // 子窗口尚未就绪（WebView2 初始化异步）：快速阶段短间隔，之后低频常驻
            let wait = if attempt < FAST_ATTEMPTS { 300 } else { 60_000 };
            std::thread::sleep(Duration::from_millis(wait));
        }
    });
}

/// 非 Windows 无 WebView2/OLE 拖放路径
#[cfg(not(windows))]
pub fn register_drop_target(_app: &AppHandle) {}

/// 枚举子窗口逐个撤销旧注册并挂自有目标，返回成功个数
#[cfg(windows)]
unsafe fn try_register_children(app: &AppHandle) -> usize {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;

    let Some(win) = app.get_webview_window("stagedrop") else {
        return 0;
    };
    let Ok(container) = win.hwnd() else {
        return 0;
    };
    let mut registered = 0usize;
    let mut callback = |child: HWND| -> bool {
        // 闭包体自身是安全上下文，unsafe fn 调用须显式块
        if unsafe { register_on_hwnd(app, child) } {
            registered += 1;
        }
        true
    };
    let mut trait_obj: &mut dyn FnMut(HWND) -> bool = &mut callback;
    let closure_pointer_pointer: *mut std::ffi::c_void =
        unsafe { std::mem::transmute(&mut trait_obj) };
    let lparam = LPARAM(closure_pointer_pointer as _);
    unsafe {
        let _ = EnumChildWindows(Some(container), Some(enumerate_cb), lparam);
    }
    registered
}

#[cfg(windows)]
unsafe extern "system" fn enumerate_cb(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let closure =
        unsafe { &mut *(lparam.0 as *mut std::ffi::c_void as *mut &mut dyn FnMut(HWND) -> bool) };
    closure(hwnd).into()
}

/// 单个子窗口：撤销 wry/WebView2 的注册，挂自有目标（后者赢得 drop 命中）
#[cfg(windows)]
unsafe fn register_on_hwnd(app: &AppHandle, hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::System::Ole::{RegisterDragDrop, RevokeDragDrop};

    let target: IDropTarget = StageDropTarget {
        app: app.clone(),
        has_files: AtomicBool::new(false),
    }
    .into();
    // edition 2024 unsafe_op_in_unsafe_fn：unsafe fn 体内的不安全操作仍须显式块
    unsafe {
        // 无注册时撤销返回错误属正常，忽略；注册失败（极少）交由上层计数
        let _ = RevokeDragDrop(hwnd);
        // RegisterDragDrop 自持 AddRef，本地引用释放不影响目标存活
        RegisterDragDrop(hwnd, &target).is_ok()
    }
}

/// 从 COM 目标提取 CF_HDROP 路径（返回路径与 HDROP；调用方负责 DragFinish）
#[cfg(windows)]
fn extract_drop_paths(
    data_obj: windows::core::Ref<'_, windows::Win32::System::Com::IDataObject>,
) -> (Vec<String>, Option<windows::Win32::UI::Shell::HDROP>) {
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Some(data) = data_obj.as_ref() else {
        return (Vec::new(), None);
    };
    let Ok(medium) = (unsafe { data.GetData(&format) }) else {
        return (Vec::new(), None);
    };
    let hdrop = HDROP(unsafe { medium.u.hGlobal }.0 as _);
    let count = unsafe { DragQueryFileW(hdrop, 0xFFFF_FFFF, None) };
    let mut paths = Vec::with_capacity(count as usize);
    for i in 0..count {
        let len = unsafe { DragQueryFileW(hdrop, i, None) } as usize;
        let mut buf = vec![0u16; len + 1];
        unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) };
        paths.push(String::from_utf16_lossy(&buf[..len]));
    }
    (paths, Some(hdrop))
}

/// 自有拖放目标：CF_HDROP 全收，效果恒 COPY（根治禁止图标）
#[cfg(windows)]
#[windows::core::implement(IDropTarget)]
struct StageDropTarget {
    app: AppHandle,
    /// DragEnter 是否带文件（DragOver/Leave/Drop 的分支依据；OLE 回调均在
    /// 注册线程=主线程，无跨线程访问）
    has_files: AtomicBool,
}

#[cfg(windows)]
#[allow(non_snake_case)]
impl windows::Win32::System::Ole::IDropTarget_Impl for StageDropTarget_Impl {
    fn DragEnter(
        &self,
        pDataObj: windows::core::Ref<'_, windows::Win32::System::Com::IDataObject>,
        _grfKeyState: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        _pt: &windows::Win32::Foundation::POINTL,
        pdwEffect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows::core::Result<()> {
        // DragEnter 只取「是否带文件」判定，HDROP 用完即释放——GetData 返回的
        // STGMEDIUM 归调用方所有，不 DragFinish 即泄漏全局内存块（会话内反复
        // 进出窗口会累积；Drop 路径同款处理见下）
        let (paths, hdrop) = extract_drop_paths(pDataObj);
        if let Some(hdrop) = hdrop {
            unsafe { windows::Win32::UI::Shell::DragFinish(hdrop) };
        }
        let has = !paths.is_empty();
        self.has_files.store(has, Ordering::Relaxed);
        HOVERING.store(has, Ordering::Relaxed);
        // 诊断：展开失效排查——hot 内容出现即证明本回调在跑，这里钉住 has
        super::diag::log(&format!("stagedrop: drag enter has={has}"));
        unsafe {
            *pdwEffect = if has {
                windows::Win32::System::Ole::DROPEFFECT_COPY
            } else {
                windows::Win32::System::Ole::DROPEFFECT_NONE
            };
        }
        if has {
            // QQ 式悬停展开：窗口右下角锚定放大，光标仍在卡片区内
            layout_bar(&self.app, true);
            let _ = self.app.emit_to("stagedrop", "zap://stagedrop-hot", true);
        }
        Ok(())
    }

    fn DragOver(
        &self,
        _grfKeyState: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        _pt: &windows::Win32::Foundation::POINTL,
        pdwEffect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            *pdwEffect = if self.has_files.load(Ordering::Relaxed) {
                windows::Win32::System::Ole::DROPEFFECT_COPY
            } else {
                windows::Win32::System::Ole::DROPEFFECT_NONE
            };
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.has_files.store(false, Ordering::Relaxed);
        HOVERING.store(false, Ordering::Relaxed);
        super::diag::log("stagedrop: drag leave");
        layout_bar(&self.app, false);
        let _ = self.app.emit_to("stagedrop", "zap://stagedrop-hot", false);
        Ok(())
    }

    fn Drop(
        &self,
        pDataObj: windows::core::Ref<'_, windows::Win32::System::Com::IDataObject>,
        _grfKeyState: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        _pt: &windows::Win32::Foundation::POINTL,
        pdwEffect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.has_files.store(false, Ordering::Relaxed);
        HOVERING.store(false, Ordering::Relaxed);
        // 收回细条再入库：反馈文案显示在细条里
        layout_bar(&self.app, false);
        let _ = self.app.emit_to("stagedrop", "zap://stagedrop-hot", false);
        let (paths, hdrop) = extract_drop_paths(pDataObj);
        if let Some(hdrop) = hdrop {
            unsafe { windows::Win32::UI::Shell::DragFinish(hdrop) };
        }
        super::diag::log(&format!("stagedrop: drop paths={}", paths.len()));
        unsafe {
            *pdwEffect = windows::Win32::System::Ole::DROPEFFECT_COPY;
        }
        if !paths.is_empty() {
            // OLE 回调在主线程：入库压后台线程，不阻塞拖拽循环
            let app = self.app.clone();
            std::thread::spawn(move || crate::handle_stage_drop(&app, paths));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(added: i64, total: i64, failed: &[&str]) -> super::super::stage::StageAddOutcome {
        super::super::stage::StageAddOutcome {
            added,
            total,
            failed: failed.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn 反馈文案按结果分桶() {
        assert_eq!(
            feedback_text(&outcome(3, 12, &[])),
            "已放入 3 条 · 共 12 条"
        );
        assert_eq!(
            feedback_text(&outcome(0, 5, &[])),
            "已在中转站（已置顶）· 共 5 条",
            "纯重复放入 = 置顶不是新增"
        );
        assert_eq!(
            feedback_text(&outcome(2, 7, &["路径不存在：x"])),
            "已放入 2 条，1 条失败"
        );
        assert_eq!(
            feedback_text(&outcome(0, 5, &["路径不存在：x"])),
            "路径不存在：x",
            "全部失败时展示首个失败原因"
        );
    }

    /// STATE 是进程级全局：碰它的测试共用一把锁串行化（代际号会被并行测试互改）
    static STATE_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 代际守卫：begin 期间开了新会话，旧结果不得覆盖新会话状态
    #[test]
    fn 过期代际的完成结果作废() {
        let _g = STATE_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let generation = begin_processing();
        // 模拟新会话递增代际
        {
            let mut st = STATE.lock().unwrap();
            st.generation += 1;
        }
        finish_processing(generation, "已放入 1 条 · 共 1 条".into(), false);
        assert!(
            current_text().is_none_or(|t| t == PROCESSING_TEXT),
            "过期代际的结果不应覆盖处理态"
        );
        // 同代际正常写入
        let generation = begin_processing();
        finish_processing(generation, "已放入 1 条 · 共 1 条".into(), false);
        assert_eq!(current_text().as_deref(), Some("已放入 1 条 · 共 1 条"));
        // 清场（全局状态，勿影响其他测试）
        STATE.lock().unwrap().feedback = None;
    }

    /// 失败反馈停留时长分档：失败 3s / 成功 1.2s（读不完失败原因是真实痛点）
    #[test]
    fn 失败反馈停留更久() {
        let _g = STATE_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let g = begin_processing();
        let before = Instant::now();
        finish_processing(g, "路径不存在：x".into(), true);
        let until = STATE.lock().unwrap().feedback.as_ref().unwrap().until;
        assert!(until.duration_since(before) >= Duration::from_millis(FEEDBACK_ERR_MS));
        STATE.lock().unwrap().feedback = None;

        let g = begin_processing();
        let before = Instant::now();
        finish_processing(g, "已放入 1 条 · 共 1 条".into(), false);
        let until = STATE.lock().unwrap().feedback.as_ref().unwrap().until;
        assert!(until.duration_since(before) < Duration::from_millis(FEEDBACK_ERR_MS));
        STATE.lock().unwrap().feedback = None;
    }

    /// 轮询决策纯函数的情形覆盖：抖动宽限/会话结束/反馈驻留/到期收条。
    /// 时间字段以 now 为锚的毫秒偏移给出（负 = 过去）
    #[test]
    fn 轮询决策情形覆盖() {
        let now = Instant::now();
        let at = |ms: i64| {
            if ms >= 0 {
                now + Duration::from_millis(ms as u64)
            } else {
                now - Duration::from_millis((-ms) as u64)
            }
        };
        let mk = |session: bool,
                  last_seen: Option<i64>,
                  gone: Option<i64>,
                  fb_until: Option<i64>| DropState {
            session_active: session,
            last_seen: last_seen.map(at),
            gone_since: gone.map(at),
            feedback: fb_until.map(|u| Feedback {
                text: "t".into(),
                until: at(u),
            }),
            generation: 0,
        };

        // 拖拽信号出现且无会话 → 开新会话
        assert!(matches!(
            decide(&mk(false, None, None, None), true, false, now),
            Poll::Start { suppressed: false }
        ));
        // 中转站视图展开中的新会话 → 记账但不出条（视图内拖入即落点）
        assert!(matches!(
            decide(&mk(false, None, None, None), true, true, now),
            Poll::Start { suppressed: true }
        ));
        // 拖拽中反馈未到期 → 保持
        assert!(matches!(
            decide(&mk(true, Some(0), None, Some(1200)), true, false, now),
            Poll::Settle
        ));
        // 拖拽中反馈已到期 → 复位文案（长拖拽不残留上一轮结果）
        assert!(matches!(
            decide(&mk(true, Some(0), None, Some(-1)), true, false, now),
            Poll::ResetStale
        ));
        // 信号消失 0.5s（< 1.2s 去抖）→ 宽限不动（躲猫猫修复的核心）
        assert!(matches!(
            decide(&mk(true, Some(-500), None, None), false, false, now),
            Poll::Hold
        ));
        // 信号消失 1.3s（≥ 1.2s）→ 会话结束
        assert!(matches!(
            decide(&mk(true, Some(-1300), None, None), false, false, now),
            Poll::End
        ));
        // 无会话反馈未到期 → 保持展示
        assert!(matches!(
            decide(&mk(false, None, Some(-100), Some(600)), false, false, now),
            Poll::Settle
        ));
        // 无会话反馈已过期 → 到期即收（反馈存在是「到期即收」的充分条件）
        assert!(matches!(
            decide(&mk(false, None, Some(-100), Some(-1)), false, false, now),
            Poll::Hide
        ));
        // 无会话无反馈，驻留不足 200ms → 保持
        assert!(matches!(
            decide(&mk(false, None, Some(-100), None), false, false, now),
            Poll::Settle
        ));
        // 无会话无反馈，驻留已过 → 收条
        assert!(matches!(
            decide(&mk(false, None, Some(-300), None), false, false, now),
            Poll::Hide
        ));
    }

    /// 几何纯函数的公共工作区：left/top 非零防「恰好以原点为锚」掩盖坐标向错误
    const WR: WorkRect = WorkRect {
        left: 100,
        top: 200,
        right: 2100,
        bottom: 1300,
    };
    const ALL_CORNERS: [Corner; 4] = [
        Corner::BottomRight,
        Corner::BottomLeft,
        Corner::TopRight,
        Corner::TopLeft,
    ];

    #[test]
    fn 停靠角解析往返与坏值拒绝() {
        for s in ["bottom_right", "bottom_left", "top_right", "top_left"] {
            assert_eq!(Corner::parse(s).unwrap().as_str(), s);
        }
        assert!(Corner::parse("center").is_none());
        assert!(Corner::parse("").is_none());
        // 非法值不改动当前角（坏存档/脏数据保持原值）
        set_position_setting("top_left");
        set_position_setting("nonsense");
        assert_eq!(position_setting(), "top_left");
        set_position_setting("bottom_right");
        assert_eq!(position_setting(), "bottom_right");
    }

    #[test]
    fn 四角原点都在工作区内且互异() {
        let m = 12;
        let mut origins = Vec::new();
        for c in ALL_CORNERS {
            let (x, y) = corner_origin(c, WR, 280, 44, m);
            assert!(x >= WR.left + m && x + 280 <= WR.right, "x 出界: {c:?}");
            assert!(y >= WR.top + m && y + 44 <= WR.bottom, "y 出界: {c:?}");
            origins.push((x, y));
        }
        // 右下角回归锚：贴工作区右/下边距
        assert_eq!(origins[0], (2100 - 280 - 12, 1300 - 44 - 12));
        // 四点互异 = 公式真分流（若恒同值则四角设置形同虚设）
        assert_eq!(
            origins
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            4
        );
    }

    #[test]
    fn 展开卡包含细条矩形_四角皆然() {
        // 悬停展开锚定正确性的几何核心：光标落在细条内 ⇒ 也落在展开卡内
        // （否则 DragEnter 展开 → 光标出窗 → DragLeave 收条 → 抖动循环）
        let m = 12;
        for c in ALL_CORNERS {
            let (bx, by) = corner_origin(c, WR, 280, 44, m);
            let (cx, cy) = corner_origin(c, WR, 300, 264, m);
            assert!(cx <= bx, "卡左越条左: {c:?}");
            assert!(cy <= by, "卡上越条上: {c:?}");
            assert!(cx + 300 >= bx + 280, "卡右不及条右: {c:?}");
            assert!(cy + 264 >= by + 44, "卡下不及条下: {c:?}");
        }
    }

    #[test]
    fn 起点误落规避按垂直半区定向() {
        let m = 12;
        // 下角：条内起点 → 上移一个条位
        let (x, y) = corner_origin(Corner::BottomRight, WR, 280, 44, m);
        assert_eq!(
            avoid_drag_origin(Corner::BottomRight, x, y, 280, 44, m, (x + 10, y + 10)),
            (x, y - 44 - m)
        );
        // 顶角：条内起点 → 下移一个条位（原实现只有下角语义，四角化后对称补齐）
        let (tx, ty) = corner_origin(Corner::TopLeft, WR, 280, 44, m);
        assert_eq!(
            avoid_drag_origin(Corner::TopLeft, tx, ty, 280, 44, m, (tx + 10, ty + 10)),
            (tx, ty + 44 + m)
        );
        // 条外起点：原位不动（拖拽起点不在角落时不折腾条子位置）
        assert_eq!(
            avoid_drag_origin(Corner::BottomRight, x, y, 280, 44, m, (WR.left, WR.top)),
            (x, y)
        );
    }
}
