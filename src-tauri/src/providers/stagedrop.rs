//! 全局拖拽落区条：任何应用开始 shell 拖拽时，光标拖近所在显示器工作区的
//! 停靠角（设置项 stagedrop_position，四角可选默认右下；邻近门槛见下）
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
//! **非文件拖拽误弹条（2026-11-07 评审 P2）→ 邻近角出条收口（方案 A，同日
//! 拍板实施）**：检测信号语义是「源端注册了拖影的 OLE 拖拽会话」，与内容
//! 无关——浏览器等现代应用拖选中文字也产 SysDragImage；而内容在出条前
//! 不可知（外部进程拿不到进行中拖拽的 IDataObject——无公开 API，数据对象
//! 只在 DragEnter 命中已注册 IDropTarget 时才到达；全屏透明接手窗口亦不
//! 可行，透明即鼠标穿透）。故出条不问内容、只问去向：会话开始只记账，
//! 光标距停靠角细条矩形 ≤200px（逻辑值随显示器缩放）才出条、≥260px 撤条
//! （双阈值迟滞带防边界徘徊闪灼），OLE 悬停直接视为在角（展开卡会把距离
//! 撑大）——不奔向角落的拖拽（文字/框选等）全程静音，文件拖拽本就要拖
//! 到角上，出条时机即需要它的时机。代价 = 拖拽初期不再有「可放入」预告
//! （2026-11-07 用户拍板接受）。检测信号语义与出条前过滤不可行的论证见本模块上文。
//!
//! **drop 接收走自有 IDropTarget**（本文件 StageDropTarget）：wry 给 WebView2
//! 设的 AllowExternalDrop=false 路径在该窗口拒绝外部拖放（真机实测光标恒为
//! 禁止、Drop 永不达），故在 WebView2 子窗口上撤销 wry 注册、挂自有目标——
//! 文件拖拽恒 COPY、非文件拖拽 NONE（诚实拒绝），悬停/离开发 zap://stagedrop-hot 驱动前端高亮，Drop 直取
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
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

/// 运行时开关：启动从 appconfig 加载、设置保存时同步
pub static STAGEDROP_ENABLED: AtomicBool = AtomicBool::new(true);

/// 拖入松手后自动呼出中转站面板（设置开关，默认关）：open_stage_panel 的
/// 门控，关闭时放入结果仍由条内反馈呈现（成功 1.2s / 失败 3s）。启动从
/// appconfig 加载、设置保存时同步
pub static STAGEDROP_OPEN_PANEL: AtomicBool = AtomicBool::new(false);

/// 落区条设置三项的值对象（评审 P1-18 owner 链路）：load/apply 的载体，
/// position 为存档/配置文件/前端 Select 共用的字符串口径
pub struct StagedropSettings {
    pub enabled: bool,
    pub open_panel: bool,
    pub position: String,
}

/// drop 编排回调（评审 P1-17 步 0）：wry 兜底路径与自有 IDropTarget 的
/// drop 都从这里上抛给 lib.rs 的 handle_stage_drop——providers 不反向
/// 依赖 crate 根。setup 时经 register_drop_handler 注入
type DropHandler = std::sync::Arc<dyn Fn(&tauri::AppHandle, Vec<String>) + Send + Sync>;
/// pub(crate)：droptarget.rs 的 Drop 回调经此上抛（兄弟模块访问）
pub(crate) static ON_DROP: std::sync::OnceLock<DropHandler> = std::sync::OnceLock::new();

pub fn register_drop_handler(h: DropHandler) {
    let _ = ON_DROP.set(h);
}

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
/// 细条距工作区边缘的边距（逻辑 px）：layout_bar 定位与邻近角判定共用，
/// 两处漂移则判定矩形 ≠ 实际条矩形
const BAR_MARGIN_LOGICAL: f64 = 12.0;
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
    /// 本会话是否已出条（邻近角触发一次；hide_for_handoff 也置真——
    /// 防「drop 呼出面板后光标仍在角附近」的幽灵重出）
    presented: bool,
    /// 本会话是否被中转站视图压制出条（会话起点记账，中途关视图不补出）
    suppressed: bool,
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
    presented: false,
    suppressed: false,
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
/// pub(crate)：droptarget.rs 的 DragEnter/Leave/Drop 同步此标志（兄弟模块访问）
pub(crate) static HOVERING: AtomicBool = AtomicBool::new(false);

/// 中转站视图是否展开（前端 set_stage_view 同步、面板隐藏路径统一清零）。
/// 视图开着时右下角落区条不再显示：视图内拖入本身就是落点（自有
/// 「松手放入中转站」提示），角落条冗余且与面板叠放（用户拍板 2026-10-28）
pub static STAGE_VIEW_OPEN: AtomicBool = AtomicBool::new(false);

include!("stagedrop/state.rs");

/// 邻近档：Enter = 已进出条门槛，Exit = 已出撤条门槛，Band = 双门槛之间的
/// 迟滞带（保持现状——单阈值下光标在边界徘徊会反复显隐）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Near {
    Enter,
    Band,
    Exit,
}

/// 出条/撤条门槛（逻辑 px，按光标所在显示器 scale 换算物理值）：距细条
/// 矩形 ≤200 出条、≥260 撤条。数值取舍：200 约为常见笔记本屏高的 1/5，
/// 覆盖「拖向角落的最后一段」而不至于整屏乱弹；60px 迟滞带远大于单个
/// 轮询周期（80ms）内的光标位移，防徘徊闪灼
const PROXIMITY_ENTER_LOGICAL: f64 = 200.0;
const PROXIMITY_EXIT_LOGICAL: f64 = 260.0;

include!("stagedrop/decide.rs");

/// drop 接收时的可见性兜底：条子被收起后 Drop 才到达时补显（幂等，细条态）
pub fn ensure_visible(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("stagedrop")
        && !win.is_visible().unwrap_or(false)
    {
        layout_bar(app, false);
        let _ = win.show();
        BAR_SHOWN.store(true, Ordering::Relaxed);
        // 补显即视为本会话已出条：防随后的邻近判定把补显窗口当「未出条」
        // 再次 Present 重播入场动画
        STATE.lock().expect("stagedrop state poisoned").presented = true;
    }
}

/// stage_add 结果 → 展示文案（added = 真实新增数，纯重复显示「已在」更准确）
pub fn feedback_text(outcome: &super::stage::StageAddOutcome) -> String {
    if outcome.failed.is_empty() {
        if outcome.added > 0 {
            format!("已放入 {} 条 · 共 {} 条", outcome.added, outcome.total)
        } else {
            // 「固定」是 clip/stage 域的用词（R12 P2：原「已置顶」串了 vault 域）
            format!("已在中转站（已固定）· 共 {} 条", outcome.total)
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

            // 邻近档只在拖拽中有意义（出条/撤条仅发生于拖拽中）。OLE 悬停
            // 即「光标在条上」——展开卡把光标到细条矩形的距离撑大（卡高 264
            // vs 条高 44），悬停中直接按 Enter 处理，否则悬停展开卡上部会被
            // 误判撤条、条子在光标下消失
            let near = if dragging {
                if HOVERING.load(Ordering::Relaxed) {
                    Near::Enter
                } else {
                    classify_proximity(&app)
                }
            } else {
                Near::Exit
            };

            match decide(
                &st,
                dragging,
                STAGE_VIEW_OPEN.load(Ordering::Relaxed),
                super::dragout::DRAG_OUT_ACTIVE.load(Ordering::Relaxed),
                near,
                now,
            ) {
                Poll::Start {
                    suppressed,
                    present_now,
                } => {
                    // 新会话：代际递增作废旧反馈。只由图像可见性触发——左键
                    // 按住（框选/普通点击）不得误开。出条与否交给邻近角：
                    // suppressed = 中转站视图展开中，整场压制（视图内拖入即
                    // 落点；会话中途关视图也不补出条——下轮会话恢复）
                    st.session_active = true;
                    st.generation += 1;
                    st.feedback = None;
                    st.gone_since = None;
                    st.presented = false;
                    st.suppressed = suppressed;
                    drop(st);
                    if suppressed {
                        super::diag::log("stagedrop: drag session start suppressed (stage view)");
                        continue;
                    }
                    if present_now {
                        // 起点已在角附近：当场出条，观感同旧版即时出条
                        super::diag::log("stagedrop: drag session start near corner");
                        present_bar(&app);
                    } else {
                        // 远离角的新会话保持安静（邻近角出条）；顺手收掉上一
                        // 轮反馈驻留的残条——它的文案与本次拖拽无关
                        if BAR_SHOWN.load(Ordering::Relaxed) {
                            withdraw_bar(&app);
                        }
                        super::diag::log("stagedrop: drag session start away from corner");
                    }
                }
                Poll::Present => {
                    // 拖近停靠角：出条（会话内首次或撤回后再近）
                    drop(st);
                    present_bar(&app);
                }
                Poll::Withdraw => {
                    // 拖离出撤条门槛：收条，再拖回来重新走 Present 全流程
                    drop(st);
                    withdraw_bar(&app);
                }
                Poll::ResetStale => {
                    // 拖拽进行中：旧反馈文案到期复位（长拖拽不残留上一轮结果）。
                    // 只复位文案，不动会话动画（present_bar 的 start/stale 分流）
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
                    st.presented = false;
                    st.suppressed = false;
                    drop(st);
                    hide(&app);
                }
            }
        }
    });
}

include!("stagedrop/win32.rs");

/// 工作区物理矩形（layout_bar 从 MONITORINFO.rcWork 搬运；独立结构使几何
/// 纯函数不依赖 windows 类型，测试可在任意平台跑）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorkRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

include!("stagedrop/layout.rs");

// COM 目标在 droptarget.rs（审计 D4 拆出）：lib.rs 经 stagedrop::register_drop_target
// 引用，re-export 保持零改动
pub use super::droptarget::register_drop_target;

#[cfg(test)]
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
            "已在中转站（已固定）· 共 5 条",
            "纯重复放入 = 固定不是新增"
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

    /// 轮询决策纯函数的情形覆盖：邻近出条/撤条/迟滞带/抖动宽限/会话结束/
    /// 反馈驻留/到期收条。时间字段以 now 为锚的毫秒偏移给出（负 = 过去）
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
                  presented: bool,
                  suppressed: bool,
                  last_seen: Option<i64>,
                  gone: Option<i64>,
                  fb_until: Option<i64>| DropState {
            session_active: session,
            presented,
            suppressed,
            last_seen: last_seen.map(at),
            gone_since: gone.map(at),
            feedback: fb_until.map(|u| Feedback {
                text: "t".into(),
                until: at(u),
            }),
            generation: 0,
        };

        // 拖拽信号出现且无会话、远离角 → 开新会话但不出条（方案 A 核心）
        assert!(matches!(
            decide(
                &mk(false, false, false, None, None, None),
                true,
                false,
                false,
                Near::Exit,
                now
            ),
            Poll::Start {
                suppressed: false,
                present_now: false
            }
        ));
        // 拖拽信号出现且无会话、已在角附近 → 开会话并当场出条（观感同旧版）
        assert!(matches!(
            decide(
                &mk(false, false, false, None, None, None),
                true,
                false,
                false,
                Near::Enter,
                now
            ),
            Poll::Start {
                suppressed: false,
                present_now: true
            }
        ));
        // 中转站视图展开中的新会话 → 记账但整场压制（视图内拖入即落点）
        assert!(matches!(
            decide(
                &mk(false, false, false, None, None, None),
                true,
                true,
                false,
                Near::Enter,
                now
            ),
            Poll::Start {
                suppressed: true,
                present_now: false
            }
        ));
        // 会话中未出条、拖近角 → 出条
        assert!(matches!(
            decide(
                &mk(true, false, false, Some(0), None, None),
                true,
                false,
                false,
                Near::Enter,
                now
            ),
            Poll::Present
        ));
        // 会话中被视图压制 → 进门槛也不出条（会话中途关视图不补出）
        assert!(matches!(
            decide(
                &mk(true, false, true, Some(0), None, None),
                true,
                false,
                false,
                Near::Enter,
                now
            ),
            Poll::Settle
        ));
        // 会话中已出条、拖离出撤条门槛 → 撤条
        assert!(matches!(
            decide(
                &mk(true, true, false, Some(0), None, None),
                true,
                false,
                false,
                Near::Exit,
                now
            ),
            Poll::Withdraw
        ));
        // 已出条、处于迟滞带 → 保持（防边界徘徊反复显隐）
        assert!(matches!(
            decide(
                &mk(true, true, false, Some(0), None, None),
                true,
                false,
                false,
                Near::Band,
                now
            ),
            Poll::Settle
        ));
        // 拖拽中反馈未到期 → 保持
        assert!(matches!(
            decide(
                &mk(true, true, false, Some(0), None, Some(1200)),
                true,
                false,
                false,
                Near::Band,
                now
            ),
            Poll::Settle
        ));
        // 拖拽中反馈已到期 → 复位文案（长拖拽不残留上一轮结果）
        assert!(matches!(
            decide(
                &mk(true, true, false, Some(0), None, Some(-1)),
                true,
                false,
                false,
                Near::Band,
                now
            ),
            Poll::ResetStale
        ));
        // 信号消失 0.5s（< 1.2s 去抖）→ 宽限不动（躲猫猫修复的核心）
        assert!(matches!(
            decide(
                &mk(true, true, false, Some(-500), None, None),
                false,
                false,
                false,
                Near::Exit,
                now
            ),
            Poll::Hold
        ));
        // 信号消失 1.3s（≥ 1.2s）→ 会话结束
        assert!(matches!(
            decide(
                &mk(true, true, false, Some(-1300), None, None),
                false,
                false,
                false,
                Near::Exit,
                now
            ),
            Poll::End
        ));
        // 无会话反馈未到期 → 保持展示
        assert!(matches!(
            decide(
                &mk(false, false, false, None, Some(-100), Some(600)),
                false,
                false,
                false,
                Near::Exit,
                now
            ),
            Poll::Settle
        ));
        // 无会话反馈已过期 → 到期即收（反馈存在是「到期即收」的充分条件）
        assert!(matches!(
            decide(
                &mk(false, false, false, None, Some(-100), Some(-1)),
                false,
                false,
                false,
                Near::Exit,
                now
            ),
            Poll::Hide
        ));
        // 无会话无反馈，驻留不足 200ms → 保持
        assert!(matches!(
            decide(
                &mk(false, false, false, None, Some(-100), None),
                false,
                false,
                false,
                Near::Exit,
                now
            ),
            Poll::Settle
        ));
        // 无会话无反馈，驻留已过 → 收条
        assert!(matches!(
            decide(
                &mk(false, false, false, None, Some(-300), None),
                false,
                false,
                false,
                Near::Exit,
                now
            ),
            Poll::Hide
        ));
    }

    /// 邻近档双门槛：≤enter 进、≥exit 出、之间为迟滞带
    #[test]
    fn 邻近档按双门槛分带() {
        assert_eq!(proximity_class(0, 200, 260), Near::Enter);
        assert_eq!(proximity_class(200, 200, 260), Near::Enter);
        assert_eq!(proximity_class(201, 200, 260), Near::Band);
        assert_eq!(proximity_class(259, 200, 260), Near::Band);
        assert_eq!(proximity_class(260, 200, 260), Near::Exit);
        assert_eq!(proximity_class(10_000, 200, 260), Near::Exit);
    }

    /// 矩形距离：矩形内为 0，矩形外取最近边（对角走欧氏距离）
    #[test]
    fn 矩形距离内零外取最近边() {
        let bar = (0, 0, 100, 50);
        assert_eq!(rect_distance_px((50, 25), bar), 0, "矩形内为 0");
        assert_eq!(rect_distance_px((130, 25), bar), 30, "右侧水平距离");
        assert_eq!(rect_distance_px((-5, 25), bar), 5, "左侧水平距离");
        assert_eq!(rect_distance_px((50, -1), bar), 1, "上方垂直距离");
        assert_eq!(rect_distance_px((103, 54), bar), 5, "对角 3-4-5");
        assert_eq!(rect_distance_px((0, 0), bar), 0, "角点恰在矩形上");
    }

    /// 拖拽出门：DRAG_OUT_ACTIVE 期间无会话一律 Hold（R01-P1 防御——拖出会
    /// 产生 SysDragImage 的场景下，落区条弹出会把拖出临时文件误收进中转站）；
    /// 已有会话时门不介入（交给正常会话状态机收尾）
    #[test]
    fn 拖拽出门无会话不开新会话() {
        let now = Instant::now();
        let mk = || DropState {
            session_active: false,
            presented: false,
            suppressed: false,
            last_seen: None,
            gone_since: None,
            feedback: None,
            generation: 0,
        };
        // 门开 + 拖拽信号 → Hold（不开新会话、不记账）
        assert!(matches!(
            decide(&mk(), true, false, true, Near::Exit, now),
            Poll::Hold
        ));
        // 门开 + 无信号 → 照常 Hold
        assert!(matches!(
            decide(&mk(), false, false, true, Near::Exit, now),
            Poll::Hold
        ));
        // 门关 + 拖拽信号 → 正常开新会话（回归保护）
        assert!(matches!(
            decide(&mk(), true, false, false, Near::Exit, now),
            Poll::Start {
                suppressed: false,
                present_now: false
            }
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
