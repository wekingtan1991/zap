use super::SearchItem;

/// M1 兜底源：网址直达 + 三大搜索引擎，保证"输入任何东西都有回应"
pub fn fallback(query: &str) -> Vec<SearchItem> {
    let mut items = Vec::with_capacity(4);

    let looks_like_url = (query.starts_with("http://") || query.starts_with("https://"))
        || (query.contains('.') && !query.contains(' ') && query.len() > 3);
    if looks_like_url {
        let url = if query.starts_with("http") {
            query.to_string()
        } else {
            format!("https://{query}")
        };
        items.push(SearchItem {
            id: "url:direct".into(),
            title: format!("打开 {query}"),
            subtitle: url.clone(),
            kind: "url".into(),
            target: url,
            icon: None,
            score: 95.0,
        });
    }

    let encoded = urlencoding::encode(query);
    let engines = [
        ("百度", format!("https://www.baidu.com/s?wd={encoded}")),
        ("必应", format!("https://www.bing.com/search?q={encoded}")),
        ("谷歌", format!("https://www.google.com/search?q={encoded}")),
    ];
    for (i, (name, url)) in engines.iter().enumerate() {
        items.push(SearchItem {
            id: format!("web:{i}"),
            title: format!("{name}搜索：{query}"),
            subtitle: url.clone(),
            kind: "web".into(),
            target: url.clone(),
            icon: None,
            score: 10.0 - i as f32,
        });
    }
    items
}
