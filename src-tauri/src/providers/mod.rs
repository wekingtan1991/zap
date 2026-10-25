pub mod acctparse;
pub mod appconfig;
pub mod apps;
pub mod backup;
pub mod clipboard;
pub mod diag;
pub mod dpapi;
pub mod everything;
pub mod files;
pub mod jsontools;
pub mod plugin;
pub mod recent;
pub mod settings;
pub mod sysutil;
pub mod texttools;
pub mod totp;
pub mod vault;
pub mod web;
pub mod winsys;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
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
    pub score: f32,
}

/// 预计算的匹配形态：小写原文 + 拼音全拼 + 首字母串。
/// 长驻索引（应用/最近文件/设置表/窗口快照）在扫描时构建一次，
/// 搜索时只做打分——否则每次击键都要对几百个条目重转拼音。
#[derive(Clone)]
pub(crate) struct MatchForms {
    lower: String,
    full: String,
    initials: String,
}

impl MatchForms {
    pub fn new(s: &str) -> Self {
        let lower = s.to_lowercase();
        // 纯 ASCII 文本没有拼音形态
        let (full, initials) = if lower.is_ascii() {
            (String::new(), String::new())
        } else {
            pinyin_forms(s)
        };
        Self {
            lower,
            full,
            initials,
        }
    }

    /// 统一匹配打分：直连（前缀>包含>子序列）→ 拼音全拼 → 拼音首字母。
    /// query 须已 trim + 小写。
    pub fn score(&self, q_lower: &str) -> Option<f32> {
        if q_lower.is_empty() {
            return None;
        }
        if let Some(s) = score_plain(&self.lower, q_lower) {
            return Some(s);
        }
        if let Some(s) = score_plain(&self.full, q_lower) {
            return Some(s * 0.85);
        }
        if !self.initials.is_empty()
            && let Some(s) = score_plain(&self.initials, q_lower)
        {
            return Some(s * 0.7);
        }
        None
    }
}

/// query 已 trim+小写化后的打分：haystack 现场转形态。
/// 供动态内容（剪贴板行、vault 字段拼接）使用，省去每条目重复小写化 query。
pub(crate) fn score_lowered(haystack: &str, q_lower: &str) -> Option<f32> {
    MatchForms::new(haystack).score(q_lower)
}

/// 路径 → 稳定短键（FNV-1a）：条目 id 用它派生，索引重建/重启后不漂移
/// （位置 id 会随重扫变号，无法做固定/启动计数这类跨会话寻址）
pub(crate) fn stable_key(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn score_plain(hay_lower: &str, q_lower: &str) -> Option<f32> {
    if hay_lower.starts_with(q_lower) {
        return Some(100.0);
    }
    if hay_lower.contains(q_lower) {
        return Some(80.0);
    }
    let qchars: Vec<char> = q_lower.chars().collect();
    let mut qi = 0usize;
    let mut last_hit: Option<usize> = None;
    let mut gaps = 0usize;
    for (i, c) in hay_lower.chars().enumerate() {
        if qi < qchars.len() && c == qchars[qi] {
            if let Some(l) = last_hit {
                gaps += i - l - 1;
            }
            last_hit = Some(i);
            qi += 1;
        }
    }
    (qi == qchars.len()).then_some(50.0 - gaps as f32)
}

/// 生成拼音形态：全拼（非中文字符保留小写原样，跳过空白）+ 首字母串
fn pinyin_forms(s: &str) -> (String, String) {
    use pinyin::ToPinyin;
    let mut full = String::with_capacity(s.len() * 2);
    let mut initials = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii() {
            if !ch.is_whitespace() {
                let lower = ch.to_ascii_lowercase();
                full.push(lower);
                if lower.is_ascii_alphanumeric() {
                    initials.push(lower);
                }
            }
        } else if let Some(py) = ch.to_pinyin() {
            let plain = py.plain();
            full.push_str(plain);
            initials.push(plain.as_bytes()[0] as char);
        }
    }
    (full, initials)
}

pub struct SearchEngine {
    /// 共享连接：剪贴板历史 / vault / appconfig 共用（WAL 下与监听线程的写连接并存）
    clip: Mutex<Connection>,
    /// 应用源具体句柄：呼出时 refresh_if_stale 要走具体类型，不经过 trait 对象；
    /// 同一 Arc 也注册在 plugins 里，搜索时与其他源一视同仁
    apps: std::sync::Arc<apps::AppsPlugin>,
    /// 插件注册表：搜索贡献 + 剪贴板识别；顺序即结果顺序，勿随意调整
    plugins: Vec<std::sync::Arc<dyn plugin::Plugin>>,
}

impl SearchEngine {
    pub fn new(db_path: &Path) -> Self {
        let conn = clipboard::open_db(db_path).expect("打开剪贴板数据库失败");
        vault::init_table(&conn);
        appconfig::init_table(&conn);
        apps::init_stats(&conn);
        // 图标缓存随库存放，键 = lnk 路径+mtime 哈希
        let icon_cache = db_path.parent().map(|p| p.join("icons"));
        let apps = std::sync::Arc::new(apps::AppsPlugin {
            index: apps::AppIndex::scan(icon_cache.as_deref()),
        });
        // 注册顺序 = 结果展示顺序（JSON 工具识别 / apps / files / winsys /
        // 剪贴板 / 最近 / 设置 / 网页兜底）。JSON 工具只贡献识别条目、不参与
        // 常规搜索，放最前是识别条目之间的展示顺序；其余与插件化前的拼接
        // 顺序一致，保持前端展示不变
        let plugins: Vec<std::sync::Arc<dyn plugin::Plugin>> = vec![
            std::sync::Arc::new(jsontools::JsonToolsPlugin),
            std::sync::Arc::new(texttools::TextToolsPlugin),
            apps.clone(),
            std::sync::Arc::new(files::FilesPlugin),
            std::sync::Arc::new(winsys::WinsysPlugin),
            std::sync::Arc::new(clipboard::ClipboardPlugin),
            std::sync::Arc::new(recent::RecentsPlugin {
                entries: recent::scan(),
            }),
            std::sync::Arc::new(settings::SettingsPlugin),
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
        Self {
            clip: Mutex::new(conn),
            apps,
            plugins,
        }
    }

    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        appconfig::get(&conn, key)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        appconfig::set(&conn, key, value)
    }

    pub fn vault_search(&self, query: &str) -> Vec<SearchItem> {
        let conn = self.clip.lock().expect("clip db poisoned");
        vault::search(&conn, query)
    }

    /// 按行号取剪贴板原文（JSON 识别条目进工作台时回查当时的内容）
    pub fn clip_row(&self, id: i64) -> Option<String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        clipboard::get_by_id(&conn, id)
    }

    /// 后台重建应用索引（面板呼出时调用）。10 秒内已扫过则跳过，
    /// 避免连续呼出反复走目录树 + UWP 枚举；false = 本次跳过或上次未完成
    pub fn refresh_apps(&self) -> bool {
        self.apps.index.refresh_if_stale(apps::REFRESH_MIN_INTERVAL)
    }

    /// open_item 记一次应用启动（frecency 排序依据，按 target 路径计数）
    pub fn record_app_launch(&self, target: &str) {
        let conn = self.clip.lock().expect("clip db poisoned");
        apps::record_launch(&conn, target);
    }

    /// 剪贴板条目固定/取消固定（返回操作后的状态）
    pub fn clip_pin(&self, id: i64) -> Result<bool, String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        clipboard::pin_toggle(&conn, id)
    }

    pub fn vault_add(
        &self,
        title: &str,
        username: &str,
        password: &str,
        url: &str,
        notes: &str,
        totp: &str,
    ) -> Result<(), String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        vault::add(&conn, title, username, password, url, notes, totp)
    }

    pub fn vault_action(
        &self,
        id: i64,
        action: &str,
        pwd_clear_secs: i64,
    ) -> Result<String, String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        vault::action(&conn, id, action, pwd_clear_secs)
    }

    pub fn vault_get(&self, id: i64) -> Result<vault::VaultRecord, String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        vault::get(&conn, id)
    }

    // 账号字段一字排开（同 vault::update 的理由），显式放行参数个数提示
    #[allow(clippy::too_many_arguments)]
    pub fn vault_update(
        &self,
        id: i64,
        title: &str,
        username: &str,
        password: &str,
        url: &str,
        notes: &str,
        pinned: i64,
        totp: &str,
    ) -> Result<(), String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        vault::update(
            &conn, id, title, username, password, url, notes, pinned, totp,
        )
    }

    pub fn vault_import(&self, entries: Vec<vault::ParsedAccount>) -> Result<usize, String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        vault::import(&conn, entries)
    }

    /// 加密导出全部账号到备份文件，返回条目数
    pub fn vault_backup_export(&self, path: &str, passphrase: &str) -> Result<usize, String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        let entries = vault::backup_read_all(&conn)?;
        let content = backup::encrypt_backup(&entries, passphrase)?;
        std::fs::write(path, content).map_err(|e| e.to_string())?;
        Ok(entries.len())
    }

    /// 解密备份文件为预览：条目 + 每条是否与现有账号疑似重复（恢复前交前端确认）
    pub fn vault_backup_decrypt(
        &self,
        path: &str,
        passphrase: &str,
    ) -> Result<backup::BackupPreview, String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let entries = backup::decrypt_backup(&content, passphrase)?;
        let duplicates = {
            let conn = self.clip.lock().expect("clip db poisoned");
            vault::backup_duplicates(&conn, &entries)
        };
        Ok(backup::BackupPreview {
            entries,
            duplicates,
        })
    }

    /// 备份恢复：skip_duplicates 时跳过疑似重复，返回 (写入数, 跳过数)
    pub fn vault_backup_restore(
        &self,
        entries: Vec<backup::BackupEntry>,
        skip_duplicates: bool,
    ) -> Result<(usize, usize), String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        vault::backup_restore(&conn, entries, skip_duplicates)
    }

    pub fn search(&self, query: &str) -> Vec<SearchItem> {
        let q = query.trim();
        if q.is_empty() {
            let conn = self.clip.lock().expect("clip db poisoned");
            return assemble_opening(&self.plugins, &conn);
        }
        // 万能输入框：对输入框内容先跑一轮即时识别（键入/粘贴即识别），
        // 命中条目置顶于入口条目与常规结果之前
        let mut items = probe_all(&self.plugins, q);
        items.extend(run_parallel(&self.plugins, q, &self.clip));
        items
    }

    /// 设置页插件清单（仅可开关插件；核心源不列）
    pub fn plugin_states(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .filter_map(|p| {
                p.meta().map(|m| PluginInfo {
                    id: m.id.to_string(),
                    title: m.title.to_string(),
                    hint: m.hint.to_string(),
                    enabled: m.state.load(std::sync::atomic::Ordering::Relaxed),
                })
            })
            .collect()
    }

    /// 开关插件：写存档 + 同步原子量（搜索/识别热路径立即生效）
    pub fn set_plugin_enabled(&self, id: &str, on: bool) -> Result<(), String> {
        let conn = self.clip.lock().expect("clip db poisoned");
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

/// 插件并行执行：总延迟从"串行求和"变"最慢一路"（files 的 HTTP 超时 250ms 与
/// winsys 的窗口枚举不再逐个叠加在每次击键上）。输出按注册序回槽，展示顺序与
/// 串行版完全一致。needs_conn 的插件留在当前线程——rusqlite Connection 非 Sync
/// 不能跨线程；其余各占一个 scoped 线程，单个插件 panic 只损失该源（空结果）。
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
mod tests {
    /// 与生产路径同构：原文入、现场转形态（原 score_match 便捷入口已并入 MatchForms）
    fn score_match(haystack: &str, query: &str) -> Option<f32> {
        let q = query.trim().to_lowercase();
        super::MatchForms::new(haystack).score(&q)
    }

    #[test]
    fn 拼音全拼与首字母() {
        assert!(score_match("设置 settings", "shezhi").is_some());
        assert!(score_match("设置 settings", "sz").is_some());
        assert!(score_match("代理 proxy", "daili").is_some());
        assert!(score_match("代理 proxy", "dl").is_some());
        assert!(score_match("微信", "weixin").is_some());
        assert!(score_match("微信", "wx").is_some());
    }

    #[test]
    fn 英文与中文直连不受影响() {
        assert_eq!(score_match("Display", "display"), Some(100.0));
        assert!(score_match("显示", "显示").unwrap() >= 100.0);
        assert_eq!(score_match("显示", "不匹配"), None);
    }
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
                score: 0.0,
            }]
        }
    }

    /// :memory: 库走 open_db 同一建表路径
    fn mem_conn() -> Connection {
        clipboard::open_db(Path::new(":memory:")).expect("mem db")
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
                score: 0.0,
            }]
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
