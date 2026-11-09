//! 搜索引擎（自 providers/mod.rs 拆出，审计 C2）：SearchEngine 装配 + 各域
//! 搜索编排（probe / 并行 / 空面板组装 / 入口词冲突）；各域 facade 见 engine/facade.rs。
//! 兄弟 provider 与 SearchItem 经 use super::* 引入——与拆分前同一可见域

use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

use super::*;

// 各域 facade 的实现体（impl SearchEngine 分文件，零逻辑变化）
mod facade;

/// 工具 Dock 的一条入口：条目本体 + 供细条渲染的短名与入口词。
/// 短名取 PluginMeta.title（如「中转站」，比条目标题「打开中转站」更合细条），
/// 入口词取 PluginMeta.entries 供 chip 悬停提示——两者都随插件声明走，前端不硬编码
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolEntry {
    pub item: SearchItem,
    pub label: String,
    pub words: Vec<String>,
}

pub struct SearchEngine {
    /// 共享连接：剪贴板历史 / vault / appconfig 共用（WAL 下与监听线程的写连接并存）。
    /// 访问一律走 conn() 出口（run_parallel 按需传 &Mutex 除外）
    clip: Mutex<Connection>,
    /// 应用源具体句柄：呼出时 refresh_if_stale 要走具体类型，不经过 trait 对象；
    /// 同一 Arc 也注册在 plugins 里，搜索时与其他源一视同仁
    apps: std::sync::Arc<apps::AppsPlugin>,
    /// 最近文件源具体句柄：呼出时节流重扫要走具体类型（同 apps 的理由）
    recents: std::sync::Arc<recent::RecentsPlugin>,
    /// 搜索捷径源具体句柄：设置页读写清单要走具体类型（同 apps 的理由）
    shortcuts: std::sync::Arc<shortcuts::ShortcutsPlugin>,
    /// 插件注册表：搜索贡献 + 剪贴板识别；顺序即结果顺序，勿随意调整
    plugins: Vec<std::sync::Arc<dyn plugin::Plugin>>,
}

impl SearchEngine {
    pub fn new(db_path: &Path) -> Self {
        // 坏库软着陆（open_db_resilient 隔离重建）；走到 panic 只剩磁盘/权限级
        // 故障，panic hook（run 最前）会落盘死因——比静默残废可诊断。
        // 五张业务表 init 收拢在 db::open_db（审计 D1），装配期不再逐个调
        let conn = super::db::open_db_resilient(db_path)
            .unwrap_or_else(|e| panic!("数据库打开与隔离重建均失败: {e}"));
        // 图标缓存随库存放，键 = lnk 路径+mtime 哈希。
        // 索引后台化（评审 P2-02/D2）：empty 空表起步 + 后台线程首扫热替换
        // （refresh 已有 Arc swap 与 refreshing 门）——setup 不再被目录树+UWP
        // COM+图标缓存读阻塞，冷启动热键/托盘就绪优先。首扫耗时落 diag
        // （P2-01 基线纪律），图标 backfill 挂扫描完成回调（冷启动首扫只查
        // 缓存不现场提取的原语义经 refresh 空表路径保持不变）
        let icon_cache = db_path.parent().map(|p| p.join("icons"));
        let index = std::sync::Arc::new(apps::AppIndex::empty(icon_cache.as_deref()));
        index.warm();
        let apps = std::sync::Arc::new(apps::AppsPlugin { index });
        let shortcuts = std::sync::Arc::new(shortcuts::ShortcutsPlugin::new(&conn));
        let recents = std::sync::Arc::new(recent::RecentsPlugin::new());
        // 注册顺序 = 结果展示顺序（JSON 工具识别 / apps / files / winsys /
        // 剪贴板 / 最近 / 设置 / 搜索捷径 / 网页兜底）。JSON 工具只贡献识别条目、
        // 不参与常规搜索，放最前是识别条目之间的展示顺序；其余与插件化前的拼接
        // 顺序一致，保持前端展示不变；捷径排在兜底源之前（命中时优先于通用搜索）
        let plugins: Vec<std::sync::Arc<dyn plugin::Plugin>> = vec![
            std::sync::Arc::new(jsontools::JsonToolsPlugin),
            std::sync::Arc::new(texttools::TextToolsPlugin),
            // 端口查询：probe 只做字符串判定，取表只在进入工作台后发生
            std::sync::Arc::new(ports::PortsPlugin),
            std::sync::Arc::new(stage::StagePlugin),
            apps.clone(),
            std::sync::Arc::new(files::FilesPlugin),
            std::sync::Arc::new(winsys::WinsysPlugin),
            std::sync::Arc::new(clipboard::ClipboardPlugin),
            recents.clone(),
            std::sync::Arc::new(settings::SettingsPlugin),
            shortcuts.clone(),
            std::sync::Arc::new(web::WebPlugin),
        ];
        // 插件开关从存档加载到静态原子量（热路径只读原子量，不查库）
        for p in &plugins {
            if let Some(meta) = p.meta() {
                let on = appconfig::get(&conn, meta.key)
                    .map(|v| v != "0")
                    .unwrap_or(meta.default_on);
                meta.state.store(on, std::sync::atomic::Ordering::Relaxed);
            }
        }
        // 入口词声明一致性（仅开发期断言）：同一词被多个插件声明会静默双出条目。
        // 与捷径关键词的重合不在此查——那是用户数据，随 plugin_states 现算
        #[cfg(debug_assertions)]
        {
            let no_shortcuts: std::collections::HashSet<String> = Default::default();
            let conflicts = entry_conflicts(
                plugins
                    .iter()
                    .filter_map(|p| p.meta().map(|m| (m.id, m.entries))),
                &no_shortcuts,
            );
            debug_assert!(
                conflicts.is_empty(),
                "插件入口词冲突（同一词被多个插件声明，命中会双出条目）：{conflicts:?}"
            );
        }
        Self {
            clip: Mutex::new(conn),
            apps,
            recents,
            shortcuts,
            plugins,
        }
    }

    /// 共享连接锁统一出口：毒化口径（panic="abort" 下 poisoning 不可达，expect
    /// 属防御性写法——评审记录）收敛单处，改口径只动这里
    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.clip.lock().expect("clip db poisoned")
    }

    pub fn search(&self, query: &str) -> Vec<SearchItem> {
        let q = query.trim();
        if q.is_empty() {
            let conn = self.conn();
            let mut items = assemble_opening(&self.plugins, &conn);
            // 空面板即启动台：最近剪贴板之后附最常用应用（frecency，仅真实
            // 启动过的），呼出 → Enter 即启动，不必先打字
            let launches = apps::load_launch_counts(&conn);
            items.extend(self.apps.index.top_launch_items(&launches, 4));
            // 插件入口不在此拼入：已迁至结果区与底栏之间的常驻工具 Dock
            // （tool_entries 命令供给），列表只留内容，黄金地产还给剪贴板/应用。
            // 可发现性由常驻 Dock 承接，不再依赖空面板列表
            return items;
        }
        // 万能输入框：对输入框内容先跑一轮即时识别（键入/粘贴即识别），
        // 命中条目置顶于常规结果之前。probe/detect 产出的 json/tool/port/info
        // 是内容动作行，必须留在列表；插件入口只来自 fallback_entry()，经
        // tool_entries 命令供给常驻 Dock，绝不按 kind 从结果里拆分
        let mut items = probe_all(&self.plugins, q);
        items.extend(run_parallel(&self.plugins, q, &self.clip));
        items
    }

    /// app_icons 命令后端：icon_key 十进制串批量取图标 data URL（apps 段
    /// 图标按需传输）。解析失败的键静默丢弃——键由后端自己产、前端只透传，
    /// 坏串不值得报错
    pub fn app_icons(&self, keys: &[String]) -> std::collections::HashMap<String, String> {
        let parsed: Vec<u64> = keys.iter().filter_map(|k| k.parse().ok()).collect();
        self.apps
            .index
            .icon_urls(&parsed)
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    /// 工具 Dock 入口清单：按注册序收拢启用插件的 fallback_entry()，附带
    /// PluginMeta 的短名与入口词。停用插件跳过；无 meta 的核心源退回条目标题。
    /// 前端在挂载、呼出与设置页关闭后各拉一次，搜索热路径不再承担可发现性
    pub fn tool_entries(&self) -> Vec<ToolEntry> {
        tool_entries_all(&self.plugins)
    }

    /// 设置页插件清单（仅可开关插件；核心源不列）。
    /// 入口词与捷径关键词的重合随调用现算——捷径是用户数据，装配期缓存会陈旧
    pub fn plugin_states(&self) -> Vec<PluginInfo> {
        let shortcut_words: std::collections::HashSet<String> = self
            .shortcuts
            .list()
            .into_iter()
            .map(|s| s.keyword)
            .collect();
        let conflicts = entry_conflicts(
            self.plugins.iter().filter_map(|p| {
                p.meta()
                    .filter(|m| m.state.load(std::sync::atomic::Ordering::Relaxed))
                    .map(|m| (m.id, m.entries))
            }),
            &shortcut_words,
        );
        self.plugins
            .iter()
            .filter_map(|p| {
                p.meta().map(|m| PluginInfo {
                    id: m.id.to_string(),
                    title: m.title.to_string(),
                    hint: m.hint.to_string(),
                    enabled: m.state.load(std::sync::atomic::Ordering::Relaxed),
                    entries: m.entries.iter().map(|w| w.to_string()).collect(),
                    conflicts: conflicts
                        .get(m.id)
                        .map(|ws| ws.iter().map(|w| w.to_string()).collect())
                        .unwrap_or_default(),
                })
            })
            .collect()
    }

    /// 开关插件：写存档 + 同步原子量（搜索/识别热路径立即生效）
    pub fn set_plugin_enabled(&self, id: &str, on: bool) -> Result<(), String> {
        let conn = self.conn();
        for p in &self.plugins {
            if let Some(meta) = p.meta()
                && meta.id == id
            {
                appconfig::set(&conn, meta.key, if on { "1" } else { "0" })?;
                meta.state.store(on, std::sync::atomic::Ordering::Relaxed);
                return Ok(());
            }
        }
        Err(format!("未知插件：{id}"))
    }
}

/// 插件是否已被停用（无清单 = 核心源 = 恒启用）
fn plugin_off(p: &dyn plugin::Plugin) -> bool {
    p.meta()
        .is_some_and(|m| !m.state.load(std::sync::atomic::Ordering::Relaxed))
}

/// 万能输入框识别：对输入框当前内容逐插件跑 probe（停用插件跳过），
/// 命中条目整体置顶。识别是廉价字符串判定（前缀/字符集检查，256KB 上限），
/// 每次击键跑一遍没有压力
fn probe_all(plugins: &[std::sync::Arc<dyn plugin::Plugin>], q: &str) -> Vec<SearchItem> {
    let mut items = Vec::new();
    for p in plugins {
        if plugin_off(p.as_ref()) {
            continue;
        }
        items.extend(p.probe(q));
    }
    items
}

/// 工具 Dock 入口收集：按注册序取各插件的 fallback_entry()（停用插件跳过——
/// 核心源无 meta 恒过，各插件实现里再自查开关态），并附带 PluginMeta 的短名与
/// 入口词。**只认 fallback_entry()，绝不按 kind 过滤**：probe/detect 产出的
/// 同 kind 条目是内容动作行，归搜索结果，不进 Dock
fn tool_entries_all(plugins: &[std::sync::Arc<dyn plugin::Plugin>]) -> Vec<ToolEntry> {
    let mut out = Vec::new();
    for p in plugins {
        if plugin_off(p.as_ref()) {
            continue;
        }
        let meta = p.meta();
        let label = meta
            .as_ref()
            .map(|m| m.title.to_string())
            .unwrap_or_default();
        let words: Vec<String> = meta
            .as_ref()
            .map(|m| m.entries.iter().map(|w| (*w).to_string()).collect())
            .unwrap_or_default();
        for item in p.fallback_entry() {
            out.push(ToolEntry {
                label: if label.is_empty() {
                    item.title.clone()
                } else {
                    label.clone()
                },
                words: words.clone(),
                item,
            });
        }
    }
    out
}

/// 入口词冲突：返回 插件 id → 冲突入口词。冲突 = 同一词被多个声明方认领，
/// 或唯一认领方与搜索捷径关键词重合（搜索时同词双出条目）。
/// 入参是 (声明方, 词表) 序列 + 捷径关键词集合；是否纳入由调用方先过滤
/// ——停用插件的入口词不会命中，不算真实冲突。输出按词排序保证确定性
fn entry_conflicts<'a>(
    decls: impl IntoIterator<Item = (&'a str, &'a [&'a str])>,
    shortcut_words: &std::collections::HashSet<String>,
) -> std::collections::HashMap<&'a str, Vec<&'a str>> {
    let mut owners: std::collections::HashMap<&'a str, Vec<&'a str>> = Default::default();
    for (id, words) in decls {
        for w in words.iter().copied() {
            owners.entry(w).or_default().push(id);
        }
    }
    let mut out: std::collections::HashMap<&'a str, Vec<&'a str>> = Default::default();
    for (word, ids) in owners {
        if ids.len() == 1 && !shortcut_words.contains(word) {
            continue;
        }
        for id in ids {
            out.entry(id).or_default().push(word);
        }
    }
    for words in out.values_mut() {
        words.sort_unstable();
    }
    out
}

/// 插件并行执行：总延迟从"串行求和"变"最慢一路"（files 的 HTTP 超时 250ms 与
/// winsys 的窗口枚举不再逐个叠加在每次击键上）。输出按注册序回槽，展示顺序与
/// 串行版完全一致。needs_conn 的插件留在当前线程——rusqlite Connection 非 Sync
/// 不能跨线程；其余各占一个 scoped 线程。注意（2026-11-04 复核更正）：Cargo.toml
/// panic=abort 是有意取舍（防数据损坏优于局部容错）——abort 下线程 panic 即整个
/// 进程退出，下面的 join 兜底只在非 abort 构建里成立，勿把这里当 panic 隔离边界。
fn run_parallel(
    plugins: &[std::sync::Arc<dyn plugin::Plugin>],
    q: &str,
    clip: &Mutex<Connection>,
) -> Vec<SearchItem> {
    let mut slots: Vec<Option<Vec<SearchItem>>> = (0..plugins.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for (i, p) in plugins.iter().enumerate() {
            if plugin_off(p.as_ref()) || p.needs_conn() {
                continue;
            }
            handles.push((i, scope.spawn(move || p.search(q, None))));
        }
        for (i, p) in plugins.iter().enumerate() {
            if plugin_off(p.as_ref()) || !p.needs_conn() {
                continue;
            }
            // clip 锁只在真正查库的插件上短暂持有：其余源的 HTTP/窗口枚举
            // 已并行到各自线程，不会拖住共用这把锁的 vault 与设置操作
            let conn = clip.lock().expect("clip db poisoned");
            slots[i] = Some(p.search(q, Some(&conn)));
        }
        for (i, h) in handles {
            slots[i] = Some(h.join().unwrap_or_default());
        }
    });
    slots.into_iter().flatten().flatten().collect()
}

/// 设置页插件状态
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub title: String,
    pub hint: String,
    pub enabled: bool,
    /// 搜索入口词（声明化，设置页插件卡展示）
    pub entries: Vec<String>,
    /// 入口词冲突（与其他插件或搜索捷径关键词重合，设置页标红提示）
    pub conflicts: Vec<String>,
}

/// 空面板呼出的结果组装：插件对最新剪贴板的识别置顶，其后是最近剪贴板若干条。
/// 识别走库内最新一条——与列表展示同源，识别结果和用户看到的条目必然一致；
/// 记录关闭时库会变陈旧，此时宁可不识别也不误报
fn assemble_opening(
    plugins: &[std::sync::Arc<dyn plugin::Plugin>],
    conn: &Connection,
) -> Vec<SearchItem> {
    let mut items = Vec::new();
    if clipboard::CLIP_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
        && let Some((rowid, text)) = clipboard::latest(conn)
    {
        for p in plugins {
            if plugin_off(p.as_ref()) {
                continue;
            }
            items.extend(p.detect(rowid, &text));
        }
    }
    items.extend(clipboard::search(conn, "", 5));
    items
}

#[cfg(test)]
mod plugin_tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    /// 串行化对全局 CLIP_ENABLED 的读改窗口（cargo test 默认多线程并行，
    /// 开关测试与依赖开关态的断言并发会互踩）
    static CLIP_ENABLED_LOCK: Mutex<()> = Mutex::new(());

    /// 只实现识别的最小插件：把收到的 rowid/文本原样写成条目，供装配断言
    struct ProbePlugin;
    impl plugin::Plugin for ProbePlugin {
        fn search(&self, _q: &str, _c: Option<&Connection>) -> Vec<SearchItem> {
            Vec::new()
        }
        fn detect(&self, rowid: i64, text: &str) -> Vec<SearchItem> {
            vec![SearchItem {
                id: format!("probe:{rowid}"),
                title: format!("probe:{text}"),
                subtitle: String::new(),
                kind: "info".into(),
                target: String::new(),
                icon: None,
                icon_key: None,
                pinned: false,
                score: 0.0,
            }]
        }
    }

    /// :memory: 库走 open_db 同一建表路径
    fn mem_conn() -> Connection {
        crate::providers::db::open_db(Path::new(":memory:")).expect("mem db")
    }

    fn insert_clip(conn: &Connection, content: &str) -> i64 {
        conn.execute(
            "INSERT INTO clipboard_history (content, created_at) VALUES (?1, ?2)",
            rusqlite::params![content, 1000],
        )
        .expect("insert");
        conn.query_row("SELECT last_insert_rowid()", [], |r| r.get(0))
            .expect("rowid")
    }

    #[test]
    fn 空库呼出不识别只返回空列表() {
        let conn = mem_conn();
        let plugins: Vec<std::sync::Arc<dyn plugin::Plugin>> =
            vec![std::sync::Arc::new(ProbePlugin)];
        let items = super::assemble_opening(&plugins, &conn);
        assert!(items.is_empty(), "无历史时不应有识别条目：{items:?}");
    }

    #[test]
    fn 识别条目置顶且基于最新一条() {
        let _serial = CLIP_ENABLED_LOCK.lock().unwrap();
        let conn = mem_conn();
        insert_clip(&conn, "{\"a\":1}");
        insert_clip(&conn, "[1,2,3]");
        let plugins: Vec<std::sync::Arc<dyn plugin::Plugin>> =
            vec![std::sync::Arc::new(ProbePlugin)];
        let items = super::assemble_opening(&plugins, &conn);
        assert_eq!(items[0].id, "probe:2", "识别条目应置顶且携带最新行号");
        assert_eq!(items[0].title, "probe:[1,2,3]");
        assert_eq!(items.len(), 3, "识别 1 条 + 最近剪贴板 2 条");
        assert_eq!(items[1].kind, "clip");
        assert_eq!(items[2].kind, "clip");
    }

    #[test]
    fn 记录关闭时跳过识别但仍展示最近剪贴板() {
        let _serial = CLIP_ENABLED_LOCK.lock().unwrap();
        let conn = mem_conn();
        insert_clip(&conn, "{\"a\":1}");
        let plugins: Vec<std::sync::Arc<dyn plugin::Plugin>> =
            vec![std::sync::Arc::new(ProbePlugin)];
        clipboard::CLIP_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
        let items = super::assemble_opening(&plugins, &conn);
        clipboard::CLIP_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            items.iter().all(|i| i.kind == "clip"),
            "关闭记录时不应出现识别条目：{items:?}"
        );
        assert_eq!(items.len(), 1);
    }

    /// 万能输入框 probe 的循环级测试件：带可开关清单的 probe 源。
    /// 刻意不用 json/tool 真插件——它们的停用测试在别的模块翻转全局原子量，
    /// 与这里的读并发会互踩（跨模块没有共享串行锁）
    struct ProbeQPlugin {
        tag: &'static str,
        switch: &'static std::sync::atomic::AtomicBool,
    }
    static PROBE_Q_A: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
    static PROBE_Q_B: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

    impl plugin::Plugin for ProbeQPlugin {
        fn search(&self, _q: &str, _c: Option<&Connection>) -> Vec<SearchItem> {
            Vec::new()
        }
        fn meta(&self) -> Option<plugin::PluginMeta> {
            Some(plugin::PluginMeta {
                id: self.tag,
                title: self.tag,
                hint: "",
                key: "",
                default_on: true,
                state: self.switch,
                entries: &[],
            })
        }
        fn probe(&self, text: &str) -> Vec<SearchItem> {
            vec![SearchItem {
                id: format!("{}:{text}", self.tag),
                title: self.tag.to_string(),
                subtitle: String::new(),
                kind: "info".into(),
                target: "q".into(),
                icon: None,
                icon_key: None,
                pinned: false,
                score: 0.0,
            }]
        }
        fn fallback_entry(&self) -> Vec<SearchItem> {
            if self.switch.load(std::sync::atomic::Ordering::Relaxed) {
                vec![SearchItem {
                    id: format!("{}:fallback", self.tag),
                    title: self.tag.to_string(),
                    subtitle: String::new(),
                    kind: "info".into(),
                    target: String::new(),
                    icon: None,
                    icon_key: None,
                    pinned: false,
                    score: 0.0,
                }]
            } else {
                Vec::new()
            }
        }
    }

    #[test]
    fn 万能输入框probe聚合并跳过停用插件() {
        let _serial = CLIP_ENABLED_LOCK.lock().unwrap();
        PROBE_Q_A.store(true, std::sync::atomic::Ordering::Relaxed);
        PROBE_Q_B.store(false, std::sync::atomic::Ordering::Relaxed);
        let plugins: Vec<std::sync::Arc<dyn plugin::Plugin>> = vec![
            std::sync::Arc::new(ProbeQPlugin {
                tag: "qa",
                switch: &PROBE_Q_A,
            }),
            std::sync::Arc::new(ProbeQPlugin {
                tag: "qb",
                switch: &PROBE_Q_B,
            }),
        ];
        let items = probe_all(&plugins, "任意内容");
        assert_eq!(items.len(), 1, "停用插件的 probe 应被跳过");
        assert_eq!(items[0].id, "qa:任意内容");
        assert_eq!(items[0].target, "q");
        PROBE_Q_B.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn dock入口按注册序拼接且跳过停用插件() {
        let _serial = CLIP_ENABLED_LOCK.lock().unwrap();
        PROBE_Q_A.store(true, std::sync::atomic::Ordering::Relaxed);
        PROBE_Q_B.store(true, std::sync::atomic::Ordering::Relaxed);
        let plugins: Vec<std::sync::Arc<dyn plugin::Plugin>> = vec![
            std::sync::Arc::new(ProbeQPlugin {
                tag: "qa",
                switch: &PROBE_Q_A,
            }),
            std::sync::Arc::new(ProbeQPlugin {
                tag: "qb",
                switch: &PROBE_Q_B,
            }),
        ];
        let entries = tool_entries_all(&plugins);
        assert_eq!(
            entries
                .iter()
                .map(|e| e.item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["qa:fallback", "qb:fallback"],
            "Dock 入口应按注册序拼接"
        );
        // 红线：probe 产出的内容动作行（id 带 query 文本）不得混入 Dock
        assert!(
            entries.iter().all(|e| !e.item.id.contains("任意内容")),
            "Dock 入口只认 fallback_entry()，probe 内容行须留在列表"
        );
        assert_eq!(entries[0].label, "qa", "短名取 PluginMeta.title");
        PROBE_Q_B.store(false, std::sync::atomic::Ordering::Relaxed);
        let entries = tool_entries_all(&plugins);
        assert_eq!(
            entries
                .iter()
                .map(|e| e.item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["qa:fallback"],
            "停用插件的 Dock 入口应被跳过"
        );
        PROBE_Q_B.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn 入口词冲突_插件间重合与捷径重合() {
        let decls = [("a", &["alpha", "beta"][..]), ("b", &["beta", "gamma"][..])];
        // 插件间重合：beta 归 a 与 b；独占词不算冲突
        let no_shortcuts: std::collections::HashSet<String> = Default::default();
        let c = entry_conflicts(decls, &no_shortcuts);
        assert_eq!(c.get("a"), Some(&vec!["beta"]));
        assert_eq!(c.get("b"), Some(&vec!["beta"]));
        assert!(!c.contains_key("alpha") && !c.contains_key("gamma"));
        // 与捷径关键词重合：唯一认领方 gamma 也算冲突
        let mut sw = std::collections::HashSet::new();
        sw.insert("gamma".to_string());
        let c = entry_conflicts(decls, &sw);
        assert_eq!(c.get("b"), Some(&vec!["beta", "gamma"]));
        assert_eq!(c.get("a"), Some(&vec!["beta"]));
    }

    /// 带延迟的探针源：并行执行与保序断言用
    struct TagPlugin {
        tag: &'static str,
        delay_ms: u64,
    }
    impl plugin::Plugin for TagPlugin {
        fn search(&self, _q: &str, _c: Option<&Connection>) -> Vec<SearchItem> {
            if self.delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
            }
            vec![SearchItem {
                id: format!("tag:{}", self.tag),
                title: self.tag.to_string(),
                subtitle: String::new(),
                kind: "info".into(),
                target: String::new(),
                icon: None,
                icon_key: None,
                pinned: false,
                score: 0.0,
            }]
        }
    }

    /// needs_conn 探针：断言引擎真的传入了连接
    struct ConnTagPlugin {
        tag: &'static str,
    }
    impl plugin::Plugin for ConnTagPlugin {
        fn needs_conn(&self) -> bool {
            true
        }
        fn search(&self, _q: &str, c: Option<&Connection>) -> Vec<SearchItem> {
            assert!(c.is_some(), "needs_conn 插件必须拿到连接");
            vec![SearchItem {
                id: format!("tag:{}", self.tag),
                title: self.tag.to_string(),
                subtitle: String::new(),
                kind: "info".into(),
                target: String::new(),
                icon: None,
                icon_key: None,
                pinned: false,
                score: 0.0,
            }]
        }
    }

    #[test]
    fn 并行插件执行保持注册顺序且真并行() {
        let conn = Mutex::new(mem_conn());
        // 普通/需连接/普通交错注册；两个 80ms 延迟源并行 ≈ 80ms，串行则 ≥160ms
        let plugins: Vec<std::sync::Arc<dyn plugin::Plugin>> = vec![
            std::sync::Arc::new(TagPlugin {
                tag: "a",
                delay_ms: 80,
            }),
            std::sync::Arc::new(ConnTagPlugin { tag: "conn" }),
            std::sync::Arc::new(TagPlugin {
                tag: "b",
                delay_ms: 80,
            }),
        ];
        let t0 = std::time::Instant::now();
        let items = super::run_parallel(&plugins, "x", &conn);
        let elapsed = t0.elapsed();
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["tag:a", "tag:conn", "tag:b"],
            "并行执行输出应保持注册顺序"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(135),
            "两个 80ms 源应并行执行（实际 {elapsed:?}，串行会 ≥160ms）"
        );
    }
}
