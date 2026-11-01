//! 文本工具域：时间戳 / Base64 / URL / 链接清洗 / UUID / 哈希。
//! 入口 = 万能输入框识别（键入/粘贴即识别时间戳/UUID/Base64/URL 编码/可清洗 URL）、
//! 搜索入口条目（tool / 时间戳 / base64 等词命中）与剪贴板识别
//! （呼出时检测，识别条目按行号回查原文）。
//! 全部转换由 tool_convert_all 一次返回（防抖后单次 IPC），前端按卡片渲染

use md5::Md5;
use serde::Serialize;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

use super::SearchItem;

pub static TEXT_TOOLS_ON: AtomicBool = AtomicBool::new(true);

/// 单次输入上限：哈希/编码不受限会拖慢 IPC，与 JSON 工具同口径
pub const MAX_INPUT_BYTES: usize = 256 * 1024;

fn ensure_enabled() -> Result<(), String> {
    if TEXT_TOOLS_ON.load(Ordering::Relaxed) {
        Ok(())
    } else {
        Err("文本工具已停用，可在设置 → 插件中开启".into())
    }
}

pub struct TextToolsPlugin;

impl TextToolsPlugin {
    /// 入口条目：搜索命中与兜底清单共用同一构造，文案不漂移
    fn entry_item() -> SearchItem {
        SearchItem {
            id: "tool:entry".into(),
            title: "文本工具".into(),
            subtitle: "时间戳 / Base64 / URL / 链接清洗 / UUID / 哈希，粘贴文本即出结果".into(),
            kind: "tool".into(),
            // entry = 入口进入：工作台空内容等待粘贴
            target: "entry".into(),
            icon: None,
            pinned: false,
            score: 0.0,
        }
    }
}

impl super::plugin::Plugin for TextToolsPlugin {
    fn search(&self, query: &str, _conn: Option<&rusqlite::Connection>) -> Vec<SearchItem> {
        let words = self.meta().map(|m| m.entries).unwrap_or(&[]);
        if ensure_enabled().is_err() || !super::plugin::entry_hit(query, words) {
            return Vec::new();
        }
        vec![Self::entry_item()]
    }

    fn fallback_entry(&self) -> Vec<SearchItem> {
        if ensure_enabled().is_ok() {
            vec![Self::entry_item()]
        } else {
            Vec::new()
        }
    }

    fn meta(&self) -> Option<super::plugin::PluginMeta> {
        Some(super::plugin::PluginMeta {
            id: "text-tools",
            title: "文本工具",
            hint: "粘贴/键入时间戳/Base64/URL/UUID/哈希即时换算，粘贴链接出清洗后地址",
            key: "plugin_text_tools",
            default_on: true,
            state: &TEXT_TOOLS_ON,
            // 入口词声明化：搜索能命中的词都在这里，设置页据此展示
            entries: &[
                "tool",
                "文本工具",
                "时间戳",
                "base64",
                "uuid",
                "哈希",
                "hash",
                "编码",
                "解码",
                "md5",
                "sha1",
                "sha256",
                "url",
            ],
        })
    }

    fn detect(&self, clip_rowid: i64, clip_text: &str) -> Vec<SearchItem> {
        if ensure_enabled().is_err() {
            return Vec::new();
        }
        let hits = recognize(clip_text.trim());
        if hits.is_empty() {
            return Vec::new();
        }
        vec![SearchItem {
            id: format!("tool:clip:{clip_rowid}"),
            title: format!("检测到剪贴板{}", hits_title(&hits)),
            subtitle: "文本工具 · 换算/编解码/哈希，Enter 打开".into(),
            kind: "tool".into(),
            // target = 剪贴板行号：进入工作台后按行号回查当时内容
            target: clip_rowid.to_string(),
            icon: None,
            pinned: false,
            score: 0.0,
        }]
    }

    /// 万能输入框识别：与 detect 同一套 recognize 判定，
    /// target = "q"（前端以当前输入框文本为工作台初始内容）
    fn probe(&self, text: &str) -> Vec<SearchItem> {
        if ensure_enabled().is_err() {
            return Vec::new();
        }
        let hits = recognize(text.trim());
        if hits.is_empty() {
            return Vec::new();
        }
        vec![SearchItem {
            id: "tool:q".into(),
            title: format!("识别到{}", hits_title(&hits)),
            subtitle: "文本工具 · 换算/编解码/哈希，Enter 打开".into(),
            kind: "tool".into(),
            target: "q".into(),
            icon: None,
            pinned: false,
            score: 0.0,
        }]
    }
}

/// 识别命中：标签 + 即问即答的答案（时间戳给换算时刻、Base64/URL 给解码预览）。
/// 答案直接写进条目标题，最常见的「这是几点/这是什么」扫一眼即得，不用进工作台
fn recognize(text: &str) -> Vec<(&'static str, Option<String>)> {
    if text.is_empty() || text.len() > MAX_INPUT_BYTES {
        return Vec::new();
    }
    let mut hits = Vec::new();
    if is_timestamp_text(text) {
        let answer = text
            .parse::<u64>()
            .ok()
            .and_then(|n| to_local_datetime(if n > 100_000_000_000 { n } else { n * 1000 }));
        hits.push(("时间戳", answer));
    }
    if is_uuid_text(text) {
        hits.push(("UUID", None));
    }
    if is_base64_text(text) {
        use base64::Engine as _;
        let answer = base64::engine::general_purpose::STANDARD
            .decode(text)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .map(|s| preview(&s));
        hits.push(("Base64", answer));
    }
    if count_percent_escapes(text) >= 3 {
        let answer = urlencoding::decode(text)
            .ok()
            .map(|c| preview(&c))
            .filter(|d| !d.is_empty());
        hits.push(("URL 编码", answer));
    }
    // 整体是 http(s) URL 且有参数可清才出「链接清洗」——即问即答给清洗后预览，
    // 完整可复制版本在工作台「链接清洗」卡
    if let Some(cleaned) = url_clean(text) {
        hits.push(("链接清洗", Some(preview(&cleaned))));
    }
    hits
}

/// 识别条目标题段：有答案的命中即问即答（「时间戳 → 2026-10-25 19:32:00」）
fn hits_title(hits: &[(&'static str, Option<String>)]) -> String {
    hits.iter()
        .map(|(label, ans)| match ans {
            Some(a) => format!("{label} → {a}"),
            None => label.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

/// 解码答案预览：滤控制字符、压平空白，超 40 字符截断
fn preview(text: &str) -> String {
    let flat: String = text
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = flat.chars().take(40).collect();
    if flat.chars().count() > 40 {
        out.push('…');
    }
    out
}

/// 10 位秒级（2001-2286 年）或 13 位毫秒级时间戳
fn is_timestamp_text(text: &str) -> bool {
    if !text.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match text.parse::<u64>() {
        Ok(n) => {
            (1_000_000_000..=2_000_000_000).contains(&n)
                || (1_000_000_000_000..=2_000_000_000_000).contains(&n)
        }
        Err(_) => false,
    }
}

/// 标准 8-4-4-4-12 十六进制 UUID
fn is_uuid_text(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let lens = [8, 4, 4, 4, 12];
    parts
        .iter()
        .zip(lens)
        .all(|(p, l)| p.len() == l && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// 形似 Base64 且能干净解码为 UTF-8 文本（纯数字交给时间戳判定，不算 Base64）
fn is_base64_text(text: &str) -> bool {
    if text.len() < 16 || text.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if !text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        return false;
    }
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .is_some()
}

fn count_percent_escapes(text: &str) -> usize {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(2))
        .filter(|&i| {
            bytes[i] == b'%' && bytes[i + 1].is_ascii_hexdigit() && bytes[i + 2].is_ascii_hexdigit()
        })
        .count()
}

/// 追踪参数显式清单（小写）：国际广告平台 + 国内分享链高频键。
/// 收窄原则 = 只列确定性追踪键，功能性参数一律不收
/// （小红书 xsec_token 是访问必需 token、微信 __biz 是公众号身份、scene 是场景值——刻意不进清单）
const TRACKING_PARAMS: &[&str] = &[
    // Google / Microsoft 广告
    "gclid",
    "gclsrc",
    "dclid",
    "wbraid",
    "gbraid",
    // Meta
    "fbclid",
    "fb_action_ids",
    "fb_action_types",
    // 百度 / 字节营销落地
    "yclid",
    "bd_vid",
    "ttclid",
    // Instagram / Twitter 分享
    "igshid",
    "ref_src",
    "ref_url",
    // YouTube 分享追踪
    "si",
    // Mailchimp
    "mc_cid",
    "mc_eid",
    // 阿里系（淘宝/闲鱼）
    "spm",
    "spm_id_from",
    "scm",
    // B 站分享链
    "vd_source",
    "vd_channel",
    "vd_share",
    "from_spmid",
    "from_source",
    "unique_k",
    // 微信公众号文章
    "chksm",
];

/// 前缀通配：utm_（全网 Analytics 标准）与 share_（B 站/抖音/CSDN 分享链高频）。
/// share_ 若遇站内功能性参数（暂未发现），按个案移出通配收窄
const TRACKING_PREFIXES: &[&str] = &["utm_", "share_"];

/// http(s):// 前缀，返回 "://" 的结束位置。scheme 大小写不敏感
fn scheme_prefix_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.len() > 7 && bytes[..7].eq_ignore_ascii_case(b"http://") {
        return Some(7);
    }
    if bytes.len() > 8 && bytes[..8].eq_ignore_ascii_case(b"https://") {
        return Some(8);
    }
    None
}

/// 追踪参数键判定：显式清单 + 前缀通配（键名按小写比较，不做 percent 解码——
/// 追踪键名实际不以转义形态出现）
fn is_tracking_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    TRACKING_PARAMS.contains(&lower.as_str())
        || TRACKING_PREFIXES.iter().any(|p| lower.starts_with(p))
}

/// 清洗 URL 的追踪参数：无 http(s) scheme / 无 query / 无命中 → None。
/// fragment 保留原样；剩余参数保序；参数全清时不留悬空 `?`
pub fn url_clean(input: &str) -> Option<String> {
    let text = input.trim();
    let scheme_len = scheme_prefix_len(text)?;
    if text.chars().any(char::is_whitespace) {
        return None;
    }
    let rest = &text[scheme_len..];
    let (rest, fragment) = match rest.split_once('#') {
        Some((r, f)) => (r, Some(f)),
        None => (rest, None),
    };
    let (base, query) = rest.split_once('?')?;
    if base.is_empty() {
        return None;
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut removed = 0usize;
    for seg in query.split('&') {
        if seg.is_empty() {
            continue;
        }
        let key = seg.split('=').next().unwrap_or(seg);
        if is_tracking_key(key) {
            removed += 1;
        } else {
            kept.push(seg);
        }
    }
    if removed == 0 {
        return None;
    }
    let mut out = String::from(&text[..scheme_len]);
    out.push_str(base);
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    Some(out)
}

fn is_zero_width(c: char) -> bool {
    matches!(
        c,
        '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}'
    )
}

/// 去零宽字符（零宽空格/连接符/词接合符/不可见连接符/BOM），无命中 → None。
/// 对任意文本适用（从 PDF/网页复制出隐形字符粘进终端的场景）
pub fn strip_zero_width(text: &str) -> Option<String> {
    if text.chars().any(is_zero_width) {
        Some(text.chars().filter(|c| !is_zero_width(*c)).collect())
    } else {
        None
    }
}

/// 全角引号规范为半角（弯双引/弯单引 → " '），无命中 → None。
/// 「」『』是中文标点不是引号误植，保留
pub fn normalize_quotes(text: &str) -> Option<String> {
    if !text
        .chars()
        .any(|c| matches!(c, '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}'))
    {
        return None;
    }
    Some(
        text.chars()
            .map(|c| match c {
                '\u{201C}' | '\u{201D}' => '"',
                '\u{2018}' | '\u{2019}' => '\'',
                other => other,
            })
            .collect(),
    )
}

/// 一次返回全部转换结果（None = 该转换对当前输入不适用）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolBundle {
    pub text_len: usize,
    /// 10/13 位数字 → 当地日期时间
    pub ts_to_date: Option<String>,
    /// 日期时间字符串 → 秒/毫秒时间戳
    pub date_to_ts: Option<String>,
    pub base64_enc: String,
    /// Base64 解码为合法 UTF-8 文本才有值
    pub base64_dec: Option<String>,
    pub url_enc: String,
    /// 含 %XX 转义且解码为合法 UTF-8 才有值
    pub url_dec: Option<String>,
    /// 整体是 http(s) URL 且有追踪参数可清才有值（utm_*/share_*/广告与分享链高频键）
    pub url_clean: Option<String>,
    /// 含零宽字符才有值（去零宽空格/连接符/BOM 等）
    pub zero_width_stripped: Option<String>,
    /// 含全角弯引号才有值（“”‘’ → " '；「」保留）
    pub quotes_normalized: Option<String>,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}

/// 全量转换。哈希/编码用原文（不 trim——哈希语义要求逐字节一致），
/// 时间戳/解码类识别用 trim 后内容
pub fn convert_all(text: &str) -> Result<ToolBundle, String> {
    ensure_enabled()?;
    if text.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "输入 {} 超过 {} KB 上限",
            text.len(),
            MAX_INPUT_BYTES / 1024
        ));
    }
    let trimmed = text.trim();

    let ts_to_date = if is_timestamp_text(trimmed) {
        let n: u64 = trimmed.parse().unwrap_or(0);
        to_local_datetime(if n > 100_000_000_000 { n } else { n * 1000 })
    } else {
        None
    };

    let date_to_ts = parse_datetime(trimmed).map(|(secs, ms)| format!("秒 {} · 毫秒 {}", secs, ms));

    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;
    let base64_enc = b64.encode(text.as_bytes());
    let base64_dec = {
        let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.len() >= 8 && compact.len().is_multiple_of(4) {
            b64.decode(&compact)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
        } else {
            None
        }
    };

    let url_enc = urlencoding::encode(text).to_string();
    let url_dec = if count_percent_escapes(trimmed) >= 1 {
        urlencoding::decode(trimmed)
            .ok()
            .map(|c| c.to_string())
            .filter(|decoded| decoded != trimmed)
    } else {
        None
    };

    // 链接清洗按 trim 后整体判定；零宽/引号作用于全文（首尾的隐形字符同样要清）
    let url_clean = url_clean(trimmed);
    let zero_width_stripped = strip_zero_width(text);
    let quotes_normalized = normalize_quotes(text);

    Ok(ToolBundle {
        text_len: text.len(),
        ts_to_date,
        date_to_ts,
        base64_enc,
        base64_dec,
        url_enc,
        url_dec,
        url_clean,
        zero_width_stripped,
        quotes_normalized,
        md5: hex_lowercase(Md5::digest(text.as_bytes()).as_slice()),
        sha1: hex_lowercase(Sha1::digest(text.as_bytes()).as_slice()),
        sha256: hex_lowercase(Sha256::digest(text.as_bytes()).as_slice()),
    })
}

/// 生成一枚 UUID v4（版本位 4 + 变要位 10）
pub fn uuid_v4() -> Result<String, String> {
    ensure_enabled()?;
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("随机数生成失败：{e}"))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        h[0..4].concat(),
        h[4..6].concat(),
        h[6..8].concat(),
        h[8..10].concat(),
        h[10..16].concat()
    ))
}

/// 毫秒时间戳 → 当地 "YYYY-MM-DD HH:MM:SS"
fn to_local_datetime(millis: u64) -> Option<String> {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(millis as i64)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
}

/// 解析常见日期时间写法 → (秒, 毫秒)。按当地时区
fn parse_datetime(text: &str) -> Option<(i64, i64)> {
    use chrono::{NaiveDate, NaiveDateTime, TimeZone};
    let parse_naive = |s: &str| -> Option<NaiveDateTime> {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
            .ok()
            .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok())
            .or_else(|| {
                NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .ok()
                    .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
            })
    };
    let naive = parse_naive(text)?;
    let dt = chrono::Local.from_local_datetime(&naive).single()?;
    let secs = dt.timestamp();
    Some((secs, secs * 1000))
}

fn hex_lowercase(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::plugin::Plugin;
    use std::sync::Mutex;

    /// 串行化对全局开关原子量的读改窗口：停用测试翻转 TEXT_TOOLS_ON 时，
    /// 其他调 convert_all/uuid_v4 的用例并行读到 false 会互踩
    /// （同 jsontools/mod.rs plugin_tests 的既知竞态）
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// 仅取命中标签（answers 在内联答案用例里另行断言）
    fn kinds(text: &str) -> Vec<&'static str> {
        recognize(text)
            .into_iter()
            .map(|(label, _)| label)
            .collect()
    }

    #[test]
    fn 识别时间戳uuid与base64() {
        let _g = TEST_LOCK.lock().unwrap();
        assert_eq!(kinds("1791828000"), vec!["时间戳"]);
        assert_eq!(kinds("1791828000000"), vec!["时间戳"]);
        assert!(kinds("12345").is_empty(), "过短数字不判时间戳");
        assert!(kinds("999999999").is_empty(), "9 位超出下限");
        assert_eq!(kinds("550e8400-e29b-41d4-a716-446655440000"), vec!["UUID"]);
        assert!(
            kinds("550E8400E29B41D4A716446655440000").is_empty(),
            "无连字符不判 UUID"
        );
        // Base64("hello text tools") 且非纯数字
        assert_eq!(kinds("aGVsbG8gdGV4dCB0b29scw=="), vec!["Base64"]);
        // 16+ 位纯数字优先判时间戳而非 Base64
        assert!(
            kinds("1234567890123456").is_empty() || kinds("1234567890123456") == vec!["时间戳"]
        );
        assert_eq!(kinds("name%3Dzap%26ver%3D2"), vec!["URL 编码"]);
        assert!(kinds("hello world").is_empty());
    }

    #[test]
    fn 入口词命中出条目() {
        let _g = TEST_LOCK.lock().unwrap();
        for q in [
            "tool",
            "to",
            "时间戳",
            "sjc",
            "shijian",
            "base",
            "uuid",
            "md5",
            "哈希",
        ] {
            let items = TextToolsPlugin.search(q, None);
            assert_eq!(items.len(), 1, "「{q}」应命中入口");
            assert_eq!(items[0].id, "tool:entry");
            assert_eq!(items[0].kind, "tool");
            assert_eq!(items[0].target, "entry");
        }
    }

    #[test]
    fn 更长的词不再命中入口() {
        let _g = TEST_LOCK.lock().unwrap();
        for q in ["toolkit", "tools下载", "uuidgen下载", "t", ""] {
            assert!(
                TextToolsPlugin.search(q, None).is_empty(),
                "「{q}」不应命中"
            );
        }
    }

    #[test]
    fn 万能输入框识别输入内容() {
        let _g = TEST_LOCK.lock().unwrap();
        let items = TextToolsPlugin.probe("1791828000");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].target, "q", "识别条目应携带 q 源标记");
        assert!(items[0].title.contains("时间戳"));
        let items = TextToolsPlugin.probe("name%3Dzap%26ver%3D2");
        assert_eq!(items.len(), 1);
        assert!(items[0].title.contains("URL"));
        assert!(TextToolsPlugin.probe("hello").is_empty());
    }

    #[test]
    fn 识别条目内联答案免进工作台() {
        let _g = TEST_LOCK.lock().unwrap();
        use chrono::TimeZone;
        // 时间戳期望值与实现同一时区来源（Local），往返自证不脆断言
        let expected = chrono::Local
            .timestamp_millis_opt(1_791_828_000_000)
            .single()
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let items = TextToolsPlugin.probe("1791828000");
        assert_eq!(items[0].title, format!("识别到时间戳 → {expected}"));
        // detect（呼出剪贴板识别）同款带答案
        let items = TextToolsPlugin.detect(7, "1791828000");
        assert_eq!(items[0].title, format!("检测到剪贴板时间戳 → {expected}"));
        // Base64 / URL 解码预览直接可见
        let items = TextToolsPlugin.probe("aGVsbG8gdGV4dCB0b29scw==");
        assert_eq!(items[0].title, "识别到Base64 → hello text tools");
        let items = TextToolsPlugin.probe("name%3Dzap%26ver%3D2");
        assert_eq!(items[0].title, "识别到URL 编码 → name=zap&ver=2");
        // UUID 无可换算答案：仅标签
        let items = TextToolsPlugin.probe("550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(items[0].title, "识别到UUID");
    }

    #[test]
    fn 解码预览压平空白并截断() {
        let _g = TEST_LOCK.lock().unwrap();
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"hello \n\t world");
        let items = TextToolsPlugin.probe(&b64);
        assert!(
            items[0].title.contains("hello world"),
            "空白应压平：{}",
            items[0].title
        );
        let b64_long = base64::engine::general_purpose::STANDARD.encode("a".repeat(60).as_bytes());
        let items = TextToolsPlugin.probe(&b64_long);
        assert!(
            items[0].title.ends_with('…'),
            "超长预览应截断：{}",
            items[0].title
        );
    }

    #[test]
    fn 停用后入口与识别拒绝() {
        let _g = TEST_LOCK.lock().unwrap();
        TEXT_TOOLS_ON.store(false, Ordering::Relaxed);
        let search_off = TextToolsPlugin.search("tool", None);
        let probe_off = TextToolsPlugin.probe("1791828000");
        TEXT_TOOLS_ON.store(true, Ordering::Relaxed);
        assert!(search_off.is_empty());
        assert!(probe_off.is_empty());
    }

    #[test]
    fn 时间戳双向换算() {
        let _g = TEST_LOCK.lock().unwrap();
        // 固定毫秒：换算受本地时区影响，双向应自洽
        let bundle = convert_all("2026-10-24 12:00:00").unwrap();
        let date_to_ts = bundle.date_to_ts.expect("日期应可解析");
        let secs: i64 = date_to_ts
            .split('·')
            .next()
            .and_then(|s| s.trim_start_matches("秒 ").trim().parse().ok())
            .expect("应含秒字段");
        let back = convert_all(&secs.to_string()).unwrap();
        assert_eq!(
            back.ts_to_date.as_deref(),
            Some("2026-10-24 12:00:00"),
            "秒→日期应回到原点（本地时区往返）"
        );
        // 13 位毫秒输入走毫秒分支
        let ms_bundle = convert_all(&(secs * 1000).to_string()).unwrap();
        assert_eq!(ms_bundle.ts_to_date, back.ts_to_date);
    }

    #[test]
    fn base64与url编解码往返() {
        let _g = TEST_LOCK.lock().unwrap();
        let b = convert_all("hello 工具").unwrap();
        assert_eq!(b.base64_enc, "aGVsbG8g5bel5YW3");
        // 原文含中文非合法 Base64：不出解码
        assert!(b.base64_dec.is_none());
        assert_eq!(b.url_enc, "hello%20%E5%B7%A5%E5%85%B7");
        // 解码方向：合法 Base64 文本还原原文
        let b2 = convert_all("aGVsbG8g5bel5YW3").unwrap();
        assert_eq!(b2.base64_dec.as_deref(), Some("hello 工具"));
        let b3 = convert_all("name%3Dzap%26ver%3D2").unwrap();
        assert_eq!(b3.url_dec.as_deref(), Some("name=zap&ver=2"));
        // 非法 Base64 不出解码
        assert!(convert_all("hello world").unwrap().base64_dec.is_none());
    }

    #[test]
    fn 哈希与标准向量一致() {
        let _g = TEST_LOCK.lock().unwrap();
        let b = convert_all("abc").unwrap();
        assert_eq!(
            b.md5, "900150983cd24fb0d6963f7d28e17f72",
            "MD5(\"abc\") 官方向量"
        );
        assert_eq!(
            b.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d",
            "SHA1(\"abc\") 官方向量"
        );
        assert_eq!(
            b.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "SHA256(\"abc\") 官方向量"
        );
    }

    #[test]
    fn uuid生成带正确版本位() {
        let _g = TEST_LOCK.lock().unwrap();
        let u = uuid_v4().unwrap();
        assert_eq!(u.len(), 36);
        assert_eq!(u.as_bytes()[14], b'4', "版本位应为 4");
        let variant = u.as_bytes()[19];
        assert!(
            matches!(variant, b'8' | b'9' | b'a' | b'b'),
            "变要位应在 8-b"
        );
        assert_ne!(uuid_v4().unwrap(), u, "两次生成应不同");
    }

    #[test]
    fn 停用后拒绝() {
        let _g = TEST_LOCK.lock().unwrap();
        TEXT_TOOLS_ON.store(false, Ordering::Relaxed);
        let err = convert_all("x").unwrap_err();
        assert!(err.contains("已停用"));
        TEXT_TOOLS_ON.store(true, Ordering::Relaxed);
        assert!(convert_all("x").is_ok());
    }

    #[test]
    fn 链接清洗_追踪参数移除() {
        // utm_ 前缀 + 功能参数保留 + 保序
        assert_eq!(
            url_clean("https://a.com/p?id=1&utm_source=x&utm_medium=y&page=2").as_deref(),
            Some("https://a.com/p?id=1&page=2"),
        );
        // 显式清单：广告键 + B 站键 + 微信键
        assert_eq!(
            url_clean("https://www.bilibili.com/video/BV1xx?vd_source=abc&spm_id_from=333&p=2")
                .as_deref(),
            Some("https://www.bilibili.com/video/BV1xx?p=2"),
        );
        assert_eq!(
            url_clean("https://mp.weixin.qq.com/s?__biz=MzA=&chksm=abc&idx=1").as_deref(),
            Some("https://mp.weixin.qq.com/s?__biz=MzA=&idx=1"),
            "__biz 是公众号身份必须保留，chksm 清除",
        );
        assert_eq!(
            url_clean("https://a.com/watch?si=share&v=1&gclid=xyz").as_deref(),
            Some("https://a.com/watch?v=1"),
        );
        // share_ 前缀通配
        assert_eq!(
            url_clean("https://a.com/?share_source=weixin&id=3").as_deref(),
            Some("https://a.com/?id=3"),
        );
        // 键名大小写不敏感
        assert_eq!(
            url_clean("https://a.com/?UTM_Source=x&id=1").as_deref(),
            Some("https://a.com/?id=1"),
        );
        // fragment 保留；参数全清不留悬空问号
        assert_eq!(
            url_clean("https://a.com/p?utm_source=x#sec").as_deref(),
            Some("https://a.com/p#sec"),
        );
        // 无变化/不适用一律 None
        assert!(url_clean("https://a.com/?p=1").is_none(), "干净 URL 不出卡");
        assert!(url_clean("https://a.com/p").is_none(), "无 query 无可清");
        assert!(
            url_clean("https://?utm_source=x").is_none(),
            "host 段缺失拒绝"
        );
        assert_eq!(
            url_clean("https://a.com?utm_source=x").as_deref(),
            Some("https://a.com"),
            "无斜杠 host 直跟 query 是合法形态",
        );
        assert!(
            url_clean("ftp://a.com?utm_source=x").is_none(),
            "非 http(s) scheme"
        );
        assert!(url_clean("hello world").is_none(), "普通文本");
        assert!(
            url_clean("https://a.com/?utm_source=x utm_medium=y").is_none(),
            "含裸空白按非 URL 拒绝"
        );
        // 大写 scheme
        assert_eq!(
            url_clean("HTTPS://A.COM/?utm_source=x").as_deref(),
            Some("HTTPS://A.COM/"),
        );
    }

    #[test]
    fn 零宽字符与全角引号清洗() {
        assert_eq!(
            strip_zero_width("ab\u{200B}c\u{FEFF}d").as_deref(),
            Some("abcd"),
            "零宽空格与 BOM 移除"
        );
        assert!(strip_zero_width("abcd").is_none(), "无零宽字符不出卡");
        assert_eq!(
            normalize_quotes("他说“你好”与‘再见’").as_deref(),
            Some("他说\"你好\"与'再见'"),
        );
        assert!(
            normalize_quotes("「书名号」『保留』").is_none(),
            "「」『』是中文标点不转换"
        );
        assert!(normalize_quotes("plain").is_none());
        // convert_all 三字段联动：URL 且带追踪参数
        let b = convert_all("https://a.com/?utm_source=x&id=1").unwrap();
        assert_eq!(b.url_clean.as_deref(), Some("https://a.com/?id=1"));
        assert!(b.zero_width_stripped.is_none());
        assert!(b.quotes_normalized.is_none());
        let dirty = "他说“hi”\u{200B}";
        let b2 = convert_all(dirty).unwrap();
        assert_eq!(
            b2.quotes_normalized.as_deref(),
            Some("他说\"hi\"\u{200B}"),
            "引号规范不碰零宽字符——各清洗各的",
        );
        assert_eq!(b2.zero_width_stripped.as_deref(), Some("他说“hi”"));
    }

    #[test]
    fn 识别出链接清洗内联答案() {
        let _g = TEST_LOCK.lock().unwrap();
        let url = "https://a.com/p?id=1&utm_source=x";
        assert_eq!(kinds(url), vec!["链接清洗"]);
        let items = TextToolsPlugin.probe(url);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "识别到链接清洗 → https://a.com/p?id=1");
        // 干净 URL 不出识别条目（交给网页/URL 直达兜底，不抢）
        assert!(TextToolsPlugin.probe("https://a.com/p?id=1").is_empty());
        // detect 同款
        let items = TextToolsPlugin.detect(9, url);
        assert_eq!(
            items[0].title,
            "检测到剪贴板链接清洗 → https://a.com/p?id=1"
        );
    }
}
