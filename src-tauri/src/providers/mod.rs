pub mod appconfig;
pub mod apps;
pub mod clipboard;
pub mod diag;
pub mod everything;
pub mod files;
pub mod recent;
pub mod settings;
pub mod sysutil;
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
    apps: apps::AppIndex,
    clip: Mutex<Connection>,
    recents: Vec<recent::RecentEntry>,
}

impl SearchEngine {
    pub fn new(db_path: &Path) -> Self {
        let conn = clipboard::open_db(db_path).expect("打开剪贴板数据库失败");
        vault::init_table(&conn);
        appconfig::init_table(&conn);
        // 图标缓存随库存放，键 = lnk 路径+mtime 哈希
        let icon_cache = db_path.parent().map(|p| p.join("icons"));
        Self {
            apps: apps::AppIndex::scan(icon_cache.as_deref()),
            clip: Mutex::new(conn),
            recents: recent::scan(),
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

    /// 后台重建应用索引（面板呼出时调用）。10 秒内已扫过则跳过，
    /// 避免连续呼出反复走目录树 + UWP 枚举；false = 本次跳过或上次未完成
    pub fn refresh_apps(&self) -> bool {
        self.apps.refresh_if_stale(apps::REFRESH_MIN_INTERVAL)
    }

    pub fn vault_add(
        &self,
        title: &str,
        username: &str,
        password: &str,
        url: &str,
        notes: &str,
    ) -> Result<(), String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        vault::add(&conn, title, username, password, url, notes)
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

    pub fn vault_get(
        &self,
        id: i64,
    ) -> Result<(String, String, String, String, String, i64), String> {
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
    ) -> Result<(), String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        vault::update(&conn, id, title, username, password, url, notes, pinned)
    }

    pub fn vault_import(&self, entries: Vec<vault::ParsedAccount>) -> Result<usize, String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        vault::import(&conn, entries)
    }

    pub fn search(&self, query: &str) -> Vec<SearchItem> {
        let q = query.trim();
        if q.is_empty() {
            // 打开面板即见最近剪贴板，输入后各源汇入
            let conn = self.clip.lock().expect("clip db poisoned");
            return clipboard::search(&conn, "", 5);
        }
        let mut items = self.apps.search(q, 8);
        items.extend(files::search(q, 8));
        items.extend(winsys::search(q, 5));
        // clip 锁只在真正查库时短暂持有：files 的 HTTP（超时 250ms）与
        // winsys 的窗口枚举若在锁内，会拖住共用这把锁的 vault 与设置操作
        {
            let conn = self.clip.lock().expect("clip db poisoned");
            items.extend(clipboard::search(&conn, q, 6));
        }
        items.extend(recent::search(&self.recents, q, 5));
        items.extend(settings::search(q, 8));
        items.extend(web::fallback(q));
        items
    }
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
