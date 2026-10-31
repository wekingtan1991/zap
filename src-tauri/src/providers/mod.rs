pub mod acctparse;
pub mod appconfig;
pub mod apps;
pub mod backup;
pub mod clipboard;
pub mod clipimage;
pub mod db;
pub mod diag;
pub mod dpapi;
pub mod engine;
pub mod everything;
pub mod files;
pub mod jsontools;
pub mod matching;
pub mod plugin;
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
pub mod web;
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
    /// app | clip | win | recent | web | url
    pub kind: String,
    /// 应用/文件为路径，窗口为 hwnd 数值，其余为 URL，剪贴板为内容本体
    pub target: String,
    /// 应用图标 data URL（PNG），仅应用类有值；其余为 None
    pub icon: Option<String>,
    /// 置顶标记（剪贴板文本/图片、账号库共用）：结构化字段取代旧「标题加
    /// 📌 前缀」的字符串协议。serde(default) 容半新半旧会话——前端构造的
    /// 条目（打开中转站等）不带此键
    #[serde(default)]
    pub pinned: bool,
    pub score: f32,
}

// 搜索引擎在 engine.rs（审计 C2 拆出）；lib/commands/panel/settings 均经
// providers::SearchEngine / providers::PluginInfo 引用，re-export 保持零改动
pub(crate) use engine::{PluginInfo, SearchEngine};
