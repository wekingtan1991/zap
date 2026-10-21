//! 插件协议：搜索贡献与剪贴板内容识别的统一入口。
//! zap 的插件是**编译期内置**的功能模块（无动态加载）：实现 Plugin 并注册进
//! SearchEngine 即可贡献搜索结果、参与面板呼出时的剪贴板识别；前端以 kind 为键
//! 在 src/plugins/registry.tsx 声明徽标与激活行为，两端各自收敛、互不渗透。

use std::sync::atomic::AtomicBool;

use rusqlite::Connection;

use super::SearchItem;

/// 可开关插件的清单 + 运行时开关态。
/// 仅需要在设置页开关的插件实现 meta()（核心搜索源不可关，返回 None）。
/// state 是静态原子量（仿 clipboard::CLIP_ENABLED 模式）：启动时从 appconfig
/// 加载、保存设置时同步——搜索热路径只读原子量，绝不每击键查库
pub struct PluginMeta {
    /// 设置持久化 id（如 "json-tools"）
    pub id: &'static str,
    /// 设置页展示名
    pub title: &'static str,
    /// 设置页一句话用途说明
    pub hint: &'static str,
    /// appconfig 键（"0" = 停用；缺省按 default_on）
    pub key: &'static str,
    pub default_on: bool,
    pub state: &'static AtomicBool,
}

/// 统一插件协议。
/// 搜索结果顺序 = 注册顺序（前端按数组顺序展示，不做全局 score 排序），
/// 调整注册顺序会改变结果排序，需同步核对展示预期与测试。
///
/// 插件清单字段（id / 展示名 / 每插件开关）随设置页集成一并引入——
/// 在有真实消费方之前不预置，避免死 API。
pub trait Plugin: Send + Sync {
    /// 常规搜索贡献。conn 仅在 needs_conn() = true 时由引擎在短暂持锁内传入，
    /// 插件不得自行获取或长持共享连接（锁竞争军规见 SearchEngine::search）
    fn search(&self, query: &str, conn: Option<&Connection>) -> Vec<SearchItem>;
    /// 是否需要 app 数据库连接：true 的插件在 clip 锁短暂持有内执行，
    /// 与 files 的 HTTP、winsys 的窗口枚举隔离（入锁会拖住共用连接的 vault 与设置操作）
    fn needs_conn(&self) -> bool {
        false
    }
    /// 可开关插件的清单；核心源不可关，默认 None
    fn meta(&self) -> Option<PluginMeta> {
        None
    }
    /// 面板呼出（空 query）时对最新一条剪贴板的识别。
    /// 返回条目置顶展示；clip_rowid 是 clipboard_history 主键，用于按需回查原文
    /// （条目本身只带类型摘要，不携带内容——列表会明文渲染 title/subtitle）。
    /// 默认不参与识别
    fn detect(&self, clip_rowid: i64, clip_text: &str) -> Vec<SearchItem> {
        let _ = (clip_rowid, clip_text);
        Vec::new()
    }
}
