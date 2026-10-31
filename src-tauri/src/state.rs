//! 应用状态与共享常量（自 lib.rs 拆出，评审 P1-17 步 2）：AppState（Tauri
//! managed state）、直达热键动作枚举、面板窗口常量、控制台子进程隐藏启动。
//! 全部 pub(crate)——lib.rs 与各域模块（settings/panel/commands）共用

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::providers::SearchEngine;
use tauri::menu::CheckMenuItem;

pub(crate) const WINDOW_LABEL: &str = "main";
pub(crate) const PANEL_WIDTH: f64 = 720.0;
pub(crate) const PANEL_HEIGHT: f64 = 480.0;

/// 直达热键的动作：账号库 = 呼出面板并进入 pw 保护域；快速粘贴 = 不经面板，
/// 把最近一条剪贴板内容粘贴到当前前台窗口；超级面板 = 抓取选中文本出动作菜单；
/// 中转站 = 呼出面板并直接落中转站视图
#[derive(Clone, Copy)]
pub(crate) enum DirectKind {
    Vault,
    QuickPaste,
    SuperPanel,
    Stage,
}

/// 直达热键槽位单一事实源（审计 B2）：setup 启动注册（settings::register_direct_hotkeys）
/// 与 save_settings 保存循环共用同一张表，新增槽位只改这里。
/// name 供保存侧错误提示；启动侧不使用。顺序即处理顺序（保存侧槽位冲突判定依赖它）
pub(crate) const DIRECT_SLOTS: [(&str, &str, DirectKind); 4] = [
    (
        "账号库直达",
        crate::providers::appconfig::KEY_VAULT_HOTKEY,
        DirectKind::Vault,
    ),
    (
        "快速粘贴",
        crate::providers::appconfig::KEY_CLIP_HOTKEY,
        DirectKind::QuickPaste,
    ),
    (
        "超级面板",
        crate::providers::appconfig::KEY_SUPERPANEL_HOTKEY,
        DirectKind::SuperPanel,
    ),
    (
        "中转站直达",
        crate::providers::appconfig::KEY_STAGE_HOTKEY,
        DirectKind::Stage,
    ),
];

pub(crate) struct AppState {
    pub(crate) engine: std::sync::Arc<SearchEngine>,
    /// 当前生效的呼出热键（设置保存时热切换）
    pub(crate) hotkey: Mutex<String>,
    /// 直达热键表：规范化热键串 → 动作（设置保存时热更新）
    pub(crate) direct_hotkeys: Mutex<HashMap<String, DirectKind>>,
    /// 托盘「记录剪贴板历史」勾选项，设置页改动时同步勾选态
    pub(crate) clip_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    /// 面板最近一次显示的时间，用于忽略显示瞬间的焦点抖动
    pub(crate) shown_at: Mutex<Option<Instant>>,
    /// 安装流程期间保持面板可见（UAC 等弹窗会抢焦点）
    pub(crate) hold_panel: std::sync::atomic::AtomicBool,
    /// 呼出面板时的前台窗口：剪贴板条目「选中即粘贴」要交还焦点的目标
    pub(crate) prev_foreground: Mutex<Option<isize>>,
}

/// spawn 控制台子进程（cmd/powershell）：GUI 进程下不闪黑框
pub(crate) fn spawn_hidden(
    command: &mut std::process::Command,
) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn()
}
