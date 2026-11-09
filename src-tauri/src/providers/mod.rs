pub mod acctparse;
pub mod appconfig;
pub mod apps;
pub mod backup;
pub mod clipboard;
pub mod clipimage;
pub mod db;
pub mod diag;
pub mod dpapi;
pub mod dragout;
pub mod droptarget;
pub mod engine;
pub mod everything;
pub mod files;
pub mod icons;
pub mod jsontools;
pub mod lnk;
pub mod matching;
pub mod pathguard;
pub mod plugin;
pub mod ports;
pub mod recent;
pub mod settings;
pub mod settings_owner;
pub mod shortcuts;
pub mod stage;
pub mod stagedrop;
pub mod superpanel;
pub mod sysutil;
pub mod texttools;
pub mod totp;
pub mod util;
pub mod vault;
pub mod watcher;
pub mod web;
pub mod winclip;
pub mod winsys;

// 匹配引擎在 matching.rs（审计 C1 拆出）；兄弟 provider 均经 super:: 符号引用，
// 此处 re-export 保持零改动
pub(crate) use matching::{MatchForms, score_lowered, stable_key};

use serde::{Deserialize, Serialize};

/// 搜索结果条目：跨源统一的轻量结构
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// app | clip | clipimg | win | recent | web | url | file | folder |
    /// settings | stage | json | tool | vault | info | cmd（各来源 id 前缀
    /// 另见各自模块；前端镜像见 src/plugins/types.ts）
    pub kind: String,
    /// 应用/文件为路径，窗口为 hwnd 数值，其余为 URL；**剪贴板两 kind 为行号**
    /// （回查全文/全图按行号走 DB，行可能已被清理——open_item 先探存在性）
    pub target: String,
    /// 应用图标 data URL（PNG），仅应用类有值；其余为 None。
    /// apps 段已改按需传输：条目不带 data URL，icon 与 icon_key 二选一
    pub icon: Option<String>,
    /// 图标按需拉取键（apps 段）：条目只带键，前端持键向 app_icons 命令
    /// 批量拉取并按键缓存——击键路径不再每次重传 base64。u64 键以十进制
    /// 字符串过线（JSON 数字超 JS 53 位安全整数会截断）；serde(default) 容
    /// 前端构造的条目（open_item 回传等）不带键
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_key: Option<String>,
    /// 置顶标记（剪贴板文本/图片、账号库共用）：结构化字段取代旧「标题加
    /// 📌 前缀」的字符串协议。serde(default) 容半新半旧会话——前端构造的
    /// 条目（打开中转站等）不带此键
    #[serde(default)]
    pub pinned: bool,
    pub score: f32,
}

// 搜索引擎在 engine.rs（审计 C2 拆出）；lib/commands/panel/settings 均经
// providers::SearchEngine / providers::PluginInfo 引用，re-export 保持零改动
pub(crate) use engine::{PluginInfo, SearchEngine, ToolEntry};
