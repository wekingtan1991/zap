//! 插件协议：搜索贡献、剪贴板内容识别与输入框即时识别的统一入口。
//! zap 的插件是**编译期内置**的功能模块（无动态加载）：实现 Plugin 并注册进
//! SearchEngine 即可贡献搜索结果、参与面板呼出时的剪贴板识别与键入/粘贴时的
//! 万能输入框识别；前端以 kind 为键在 src/plugins/registry.tsx 声明徽标与
//! 激活行为，两端各自收敛、互不渗透。

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
    /// 搜索入口词（声明化）：插件声明自己能被哪些词搜到（entry_hit 匹配），
    /// 设置页插件卡据此展示「入口」——新增插件的入口无需再改前端
    pub entries: &'static [&'static str],
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

    /// 万能输入框：对输入框当前内容（query 本身）的即时识别，键入/粘贴即触发。
    /// 与 detect 的差异：detect 面向「呼出瞬间的剪贴板」（target = 行号，内容
    /// 以识别时刻为准），probe 面向「用户正在输入的内容」（target 约定 "q"，
    /// 前端以进入工作台时的输入框全文为初始文本）。默认不参与
    fn probe(&self, text: &str) -> Vec<SearchItem> {
        let _ = text;
        Vec::new()
    }

    /// 功能入口兜底：常规结果里除网页兜底外一无所获（或空面板呼出）时，
    /// 引擎收拢各插件的入口条目附在列表尾部——可发现性不依赖用户记得
    /// 入口词或翻设置页。兜底时机不经 entry_hit 也没有 conn，实现须自查
    /// 开关态；入口条目应与 search 命中时共用同一构造，防文案漂移。默认不参与
    fn fallback_entry(&self) -> Vec<SearchItem> {
        Vec::new()
    }
}

/// 插件入口条目的命中判定：query（trim + 小写）能否装进任一入口词——
/// 与常规搜索共用 MatchForms 打分器（前缀 > 包含 > 子序列 + 拼音全拼/首字母），
/// 方向天然只取「query 不长于入口词」：jsonview / toolkit 这类更长的词是在
/// 搜别的东西，不命中、不再劫持。过短的 query（<2 字符）不出，降低单字母噪音
pub fn entry_hit(query: &str, words: &[&str]) -> bool {
    let q = query.trim().to_lowercase();
    let n = q.chars().count();
    if !(2..=16).contains(&n) {
        return false;
    }
    words
        .iter()
        .any(|w| super::MatchForms::new(w).score(&q).is_some())
}
