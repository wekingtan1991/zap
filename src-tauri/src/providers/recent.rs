//! 最近文件：扫描系统"最近使用"目录的 .lnk，按修改时间排序

use std::path::PathBuf;

use super::SearchItem;

pub struct RecentEntry {
    pub name: String,
    pub path: String,
    pub mtime: i64,
    /// 名称的匹配形态（小写/拼音），扫描时算一次
    pub forms: super::MatchForms,
}

pub fn scan() -> Vec<RecentEntry> {
    let Some(base) = std::env::var("APPDATA").ok() else {
        return Vec::new();
    };
    let dir = PathBuf::from(base).join(r"Microsoft\Windows\Recent");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut entries: Vec<RecentEntry> = rd
        .flatten()
        .filter_map(|entry| {
            let p = entry.path();
            if p.extension().is_some_and(|ext| ext == "lnk") {
                let meta = entry.metadata().ok()?;
                let mtime = meta
                    .modified()
                    .ok()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?
                    .as_millis() as i64;
                let name = p.file_stem()?.to_string_lossy().to_string();
                let forms = super::MatchForms::new(&name);
                Some(RecentEntry {
                    name,
                    path: p.to_string_lossy().to_string(),
                    mtime,
                    forms,
                })
            } else {
                None
            }
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.mtime));
    entries
}

pub fn search(entries: &[RecentEntry], query: &str, limit: usize) -> Vec<SearchItem> {
    let q = query.trim().to_lowercase();
    let mut scored: Vec<(f32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            e.forms
                .score(&q)
                .map(|s| (s - e.name.chars().count() as f32 * 0.05, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);

    scored
        .into_iter()
        .map(|(score, i)| {
            let e = &entries[i];
            SearchItem {
                id: format!("recent:{i}"),
                title: e.name.clone(),
                subtitle: "最近使用".into(),
                kind: "recent".into(),
                target: e.path.clone(),
                icon: None,
                score: score * 0.5 - 0f32,
            }
        })
        .collect()
}

/// 最近文件源插件适配：启动时 scan() 一次的长驻索引，随插件注册进引擎
pub struct RecentsPlugin {
    pub entries: Vec<RecentEntry>,
}

impl super::plugin::Plugin for RecentsPlugin {
    fn search(&self, query: &str, _conn: Option<&rusqlite::Connection>) -> Vec<SearchItem> {
        search(&self.entries, query, 5)
    }
}
