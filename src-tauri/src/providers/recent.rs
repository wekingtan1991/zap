//! 最近文件：扫描系统"最近使用"目录的 .lnk，按修改时间排序

use std::path::PathBuf;

use super::SearchItem;

pub struct RecentEntry {
    pub name: String,
    pub path: String,
    pub mtime: i64,
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
                Some(RecentEntry {
                    name: p.file_stem()?.to_string_lossy().to_string(),
                    path: p.to_string_lossy().to_string(),
                    mtime,
                })
            } else {
                None
            }
        })
        .collect();
    entries.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    entries
}

pub fn search(entries: &[RecentEntry], query: &str, limit: usize) -> Vec<SearchItem> {
    let mut scored: Vec<(f32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            super::score_match(&e.name, query)
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
