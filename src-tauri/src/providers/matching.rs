//! 匹配打分引擎（自 providers/mod.rs 拆出，审计 C1）：预计算形态 + 统一打分。
//! 纯函数无状态——长驻索引（应用/最近文件/设置表/窗口快照）在扫描时构建一次
//! MatchForms，搜索时只做打分；动态内容（剪贴板行、vault 字段拼接）走
//! score_lowered 现场转形态

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
