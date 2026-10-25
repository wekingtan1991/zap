//! 自定义搜索捷径：关键词 → URL 模板（{query} 占位）。
//! 主面板输入 "gh rust clap" 时首词精确命中捷径 gh，生成「GitHub：rust clap」
//! 条目（kind=web，复用兜底源的浏览器打开链路）；裸关键词（"gh"）出占位提示。
//! 清单存 appconfig 单键 JSON，启动加载进 RwLock 缓存——搜索热路径只读缓存
//! 不查库，保存时整单写库并热替换

use std::sync::{Arc, RwLock};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::SearchItem;

/// 捷径条目：keyword 触发词（不含空白）/ name 展示名 / url 含 {query} 占位的模板
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shortcut {
    pub keyword: String,
    pub name: String,
    pub url: String,
}

/// appconfig 存储键（清单整体序列化为一个 JSON 值）
pub const KEY_SHORTCUTS: &str = "search_shortcuts";
/// 清单上限：防设置页误粘贴出超长清单
pub const MAX_SHORTCUTS: usize = 50;

/// 首启种子：开箱可发现，设置页可改可删
fn seed() -> Vec<Shortcut> {
    vec![
        Shortcut {
            keyword: "gh".into(),
            name: "GitHub".into(),
            url: "https://github.com/search?q={query}".into(),
        },
        Shortcut {
            keyword: "zh".into(),
            name: "知乎".into(),
            url: "https://www.zhihu.com/search?type=content&q={query}".into(),
        },
        Shortcut {
            keyword: "db".into(),
            name: "豆瓣".into(),
            url: "https://www.douban.com/search?q={query}".into(),
        },
    ]
}

/// 解析存档：坏 JSON 回退种子；空清单是合法状态（用户删光=腾出关键词），保留为空
fn load_list(conn: &Connection) -> Vec<Shortcut> {
    match super::appconfig::get(conn, KEY_SHORTCUTS) {
        Some(raw) => serde_json::from_str::<Vec<Shortcut>>(&raw).unwrap_or_else(|_| seed()),
        None => seed(),
    }
}

pub struct ShortcutsPlugin {
    /// 搜索热路径只读缓存；保存时热替换（读多写极少，RwLock 足够）
    entries: RwLock<Arc<Vec<Shortcut>>>,
}

impl ShortcutsPlugin {
    pub fn new(conn: &Connection) -> Self {
        Self {
            entries: RwLock::new(Arc::new(load_list(conn))),
        }
    }

    pub fn list(&self) -> Vec<Shortcut> {
        self.entries
            .read()
            .expect("shortcuts lock poisoned")
            .as_ref()
            .clone()
    }

    /// 整单保存：校验（字段非空、关键词无空白且不重复，均 trim、关键词小写归一）
    /// → 写库 → 热替换缓存。逐项报首个错误
    pub fn save(&self, conn: &Connection, list: Vec<Shortcut>) -> Result<(), String> {
        if list.len() > MAX_SHORTCUTS {
            return Err(format!("捷径最多 {MAX_SHORTCUTS} 条"));
        }
        let mut cleaned: Vec<Shortcut> = Vec::with_capacity(list.len());
        for (i, s) in list.into_iter().enumerate() {
            let keyword = s.keyword.trim().to_lowercase();
            let name = s.name.trim().to_string();
            let url = s.url.trim().to_string();
            if keyword.is_empty() || name.is_empty() || url.is_empty() {
                return Err(format!("第 {} 条：关键词、名称、URL 都不能为空", i + 1));
            }
            if keyword.chars().any(|c| c.is_whitespace()) {
                return Err(format!("第 {} 条：关键词不能含空格", i + 1));
            }
            if cleaned.iter().any(|c| c.keyword == keyword) {
                return Err(format!("第 {} 条：关键词「{keyword}」重复", i + 1));
            }
            cleaned.push(Shortcut { keyword, name, url });
        }
        let json = serde_json::to_string(&cleaned).map_err(|e| e.to_string())?;
        super::appconfig::set(conn, KEY_SHORTCUTS, &json)?;
        *self.entries.write().expect("shortcuts lock poisoned") = Arc::new(cleaned);
        Ok(())
    }

    /// 命中判定：query 首词（trim + 小写）精确等于某关键词，其余为参数。
    /// "ghost" 不会误命中 gh——首词整体比较，不是前缀
    fn match_query(&self, query: &str) -> Option<(Shortcut, String)> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }
        let mut parts = q.splitn(2, char::is_whitespace);
        let head = parts.next().unwrap_or_default().to_lowercase();
        let rest = parts.next().unwrap_or_default().trim().to_string();
        let entries = self.entries.read().expect("shortcuts lock poisoned");
        entries
            .iter()
            .find(|s| s.keyword == head)
            .map(|s| (s.clone(), rest))
    }
}

impl super::plugin::Plugin for ShortcutsPlugin {
    fn search(&self, query: &str, _conn: Option<&Connection>) -> Vec<SearchItem> {
        let Some((sc, arg)) = self.match_query(query) else {
            return Vec::new();
        };
        if arg.is_empty() {
            // 裸关键词：占位提示（info 不触发打开）
            return vec![SearchItem {
                id: format!("sc:{}", sc.keyword),
                title: format!("{}搜索", sc.name),
                subtitle: format!("接上要搜的内容，如：{} 关键词", sc.keyword),
                kind: "info".into(),
                target: String::new(),
                icon: None,
                score: 0.0,
            }];
        }
        let url = sc.url.replace("{query}", &urlencoding::encode(&arg));
        vec![SearchItem {
            id: format!("sc:{}", sc.keyword),
            title: format!("{}：{}", sc.name, arg),
            subtitle: url.clone(),
            kind: "web".into(),
            target: url,
            icon: None,
            score: 0.0,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::plugin::Plugin;

    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        super::super::appconfig::init_table(&conn);
        conn
    }

    #[test]
    fn 首词精确命中与占位条目() {
        let p = ShortcutsPlugin::new(&mem_conn());
        let items = p.search("gh rust clap", None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "web");
        assert_eq!(items[0].title, "GitHub：rust clap");
        assert_eq!(items[0].target, "https://github.com/search?q=rust%20clap");
        // 大小写不敏感；中文参数正常编码
        let items = p.search("ZH 时间戳 转换", None);
        assert_eq!(
            items[0].target,
            "https://www.zhihu.com/search?type=content&q=%E6%97%B6%E9%97%B4%E6%88%B3%20%E8%BD%AC%E6%8D%A2"
        );
        // 裸关键词：占位条目不可打开
        let items = p.search("gh", None);
        assert_eq!(items[0].kind, "info");
        assert!(items[0].subtitle.contains("如：gh 关键词"));
    }

    #[test]
    fn 首词不等于关键词不命中() {
        let p = ShortcutsPlugin::new(&mem_conn());
        // ghost / github 不是 gh——整体比较，不是前缀；与 json 入口词的劫持教训同源
        assert!(p.search("ghost story", None).is_empty());
        assert!(p.search("github rust", None).is_empty());
        assert!(p.search("", None).is_empty());
    }

    #[test]
    fn 无占位模板原样打开() {
        let conn = mem_conn();
        let p = ShortcutsPlugin::new(&conn);
        p.save(
            &conn,
            vec![Shortcut {
                keyword: "doc".into(),
                name: "文档".into(),
                url: "https://example.com/doc".into(),
            }],
        )
        .unwrap();
        let items = p.search("doc 任意", None);
        assert_eq!(items[0].target, "https://example.com/doc");
    }

    #[test]
    fn 保存校验与热替换() {
        let conn = mem_conn();
        let p = ShortcutsPlugin::new(&conn);
        // trim + 小写归一后生效
        p.save(
            &conn,
            vec![Shortcut {
                keyword: " MD ".into(),
                name: " MDN ".into(),
                url: " https://mdn.dev/?q={query} ".into(),
            }],
        )
        .unwrap();
        let items = p.search("md fetch", None);
        assert_eq!(items[0].target, "https://mdn.dev/?q=fetch");
        // 重复关键词 / 空字段 / 关键词带空格 / 超上限
        let dup = vec![
            Shortcut {
                keyword: "a".into(),
                name: "甲".into(),
                url: "https://a".into(),
            },
            Shortcut {
                keyword: "a".into(),
                name: "乙".into(),
                url: "https://b".into(),
            },
        ];
        assert!(p.save(&conn, dup).unwrap_err().contains("重复"));
        let empty = vec![Shortcut {
            keyword: "x".into(),
            name: "".into(),
            url: "https://x".into(),
        }];
        assert!(p.save(&conn, empty).unwrap_err().contains("不能为空"));
        let spaced = vec![Shortcut {
            keyword: "a b".into(),
            name: "丙".into(),
            url: "https://c".into(),
        }];
        assert!(p.save(&conn, spaced).unwrap_err().contains("空格"));
        let over = (0..MAX_SHORTCUTS + 1)
            .map(|i| Shortcut {
                keyword: format!("k{i}"),
                name: "n".into(),
                url: "https://x".into(),
            })
            .collect();
        assert!(p.save(&conn, over).unwrap_err().contains("最多"));
        // 校验失败不落库不换缓存：md 仍生效
        assert_eq!(
            p.search("md fetch", None)[0].target,
            "https://mdn.dev/?q=fetch"
        );
    }

    #[test]
    fn 坏存档回退种子_空清单保留为空() {
        let conn = mem_conn();
        super::super::appconfig::set(&conn, KEY_SHORTCUTS, "not-json").unwrap();
        let p = ShortcutsPlugin::new(&conn);
        assert_eq!(p.list().len(), 3, "坏 JSON 应回退种子");
        // 空清单是合法状态：删光后重启不应冒回种子
        super::super::appconfig::set(&conn, KEY_SHORTCUTS, "[]").unwrap();
        assert!(ShortcutsPlugin::new(&conn).list().is_empty());
    }
}
