//! 账号文本解析器（从 vault.rs 拆出）：智能导入的「粘贴文本 → 结构化账号」。
//! 多格式启发式识别（键值对/标签行/URL/斜杠对/手机号），识别不了的细节
//! 交给前端可编辑预览修正

/// 智能导入的单条解析结果（可编辑预览后批量入库）
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParsedAccount {
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
}

const LABEL_USER: &[&str] = &[
    "账号",
    "用户名",
    "用户",
    "登录名",
    "账号名",
    "user",
    "username",
    "account",
    "login",
];
const LABEL_PASS: &[&str] = &["密码", "口令", "password", "pwd", "pass"];
const LABEL_URL: &[&str] = &["接入地址", "地址", "链接", "网址", "url", "site", "link"];

fn label_field(line_lower: &str) -> Option<&'static str> {
    if LABEL_URL.contains(&line_lower) {
        return Some("url");
    }
    if LABEL_USER.contains(&line_lower) {
        return Some("user");
    }
    if LABEL_PASS.contains(&line_lower) {
        return Some("pass");
    }
    None
}

/// 同行 key: value（中英文冒号均可）
fn split_kv(line: &str) -> Option<(&'static str, &str)> {
    let idx = line.find(['：', ':'])?;
    let w = line[idx..].chars().next()?.len_utf8();
    let key = line[..idx].trim().to_lowercase();
    let val = line[idx + w..].trim();
    if key.is_empty() || val.is_empty() {
        return None;
    }
    let field = if LABEL_USER.iter().any(|k| key == *k) {
        "user"
    } else if LABEL_PASS.iter().any(|k| key == *k) {
        "pass"
    } else if LABEL_URL.iter().any(|k| key == *k) {
        "url"
    } else {
        return None;
    };
    Some((field, val))
}

fn ensure_scheme(url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("https://{url}")
    }
}

/// 域名/IP:端口/带协议，且不含空格
fn as_url(line: &str) -> Option<String> {
    if line.contains(' ') || !line.contains('.') {
        return None;
    }
    let has_scheme = line.starts_with("http://") || line.starts_with("https://");
    let host = line.split(['/', ':']).next().unwrap_or("");
    let looks_domain = host.contains('.') && host.chars().any(|c| c.is_ascii_alphabetic());
    let looks_ip = host.split('.').count() == 4
        && host
            .split('.')
            .all(|p| !p.is_empty() && p.parse::<u8>().is_ok());
    if has_scheme {
        Some(line.to_string())
    } else if looks_domain || (looks_ip && line.contains(':')) {
        Some(ensure_scheme(line))
    } else {
        None
    }
}

/// user/pass 斜杠对：两侧非空且不含空格
fn as_slash_pair(line: &str) -> Option<(String, String)> {
    let (l, r) = line.split_once('/')?;
    let (l, r) = (l.trim(), r.trim());
    if l.is_empty() || r.is_empty() || l.contains(' ') || r.contains(' ') {
        return None;
    }
    Some((l.to_string(), r.to_string()))
}

fn is_phone(line: &str) -> bool {
    line.len() == 11 && line.starts_with('1') && line.chars().all(|c| c.is_ascii_digit())
}

fn apply_field(e: &mut ParsedAccount, field: &str, val: &str) {
    match field {
        "user" => {
            if e.username.is_empty() {
                e.username = val.to_string();
            } else if e.password.is_empty() {
                e.password = val.to_string();
            }
        }
        "pass" => {
            if e.password.is_empty() {
                e.password = val.to_string();
            }
        }
        "url" if e.url.is_empty() => {
            e.url = ensure_scheme(val);
        }
        _ => {}
    }
}

fn has_any_cred(e: &ParsedAccount) -> bool {
    !e.username.is_empty() || !e.password.is_empty()
}

/// 多格式启发式解析。识别不了的细节交给可编辑预览修正。
pub fn parse_blob(text: &str) -> Vec<ParsedAccount> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut out: Vec<ParsedAccount> = Vec::new();
    let mut cur = ParsedAccount::default();

    // 收尾当前条目：没有任何凭据的条目没有导入价值
    let push_cur = |cur: &mut ParsedAccount, out: &mut Vec<ParsedAccount>| {
        if has_any_cred(cur) {
            out.push(std::mem::take(cur));
        } else {
            *cur = ParsedAccount::default();
        }
    };

    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];

        // 1. 同行 key: value
        if let Some((field, val)) = split_kv(line) {
            apply_field(&mut cur, field, val);
            i += 1;
            continue;
        }

        // 2. 独立标签行（值在下一行），如"接入地址\nhttps://…"
        if let Some(field) = label_field(&line.to_lowercase()) {
            match lines.get(i + 1) {
                Some(next)
                    if split_kv(next).is_none() && label_field(&next.to_lowercase()).is_none() =>
                {
                    apply_field(&mut cur, field, next);
                    i += 2;
                }
                _ => i += 1,
            }
            continue;
        }

        // 3. URL 行；若当前条目已有网址且带凭据，则视为下一条目的开始
        if let Some(url) = as_url(line) {
            if !cur.url.is_empty() && has_any_cred(&cur) {
                push_cur(&mut cur, &mut out);
            }
            apply_field(&mut cur, "url", &url);
            i += 1;
            continue;
        }

        // 4. user/pass 斜杠对
        if let Some((u, p)) = as_slash_pair(line) {
            apply_field(&mut cur, "user", &u);
            apply_field(&mut cur, "pass", &p);
            i += 1;
            continue;
        }

        // 5. 手机号：填入空的用户名/密码
        if is_phone(line) {
            apply_field(&mut cur, "user", line);
            i += 1;
            continue;
        }

        // 6. 其余裸行：空标题 → 当标题（去尾随冒号）；已有标题时，
        //    含中文的行视为新条目标题（密码里几乎不会出现中文），
        //    纯 ASCII 行补空的用户名/密码
        if cur.title.is_empty() {
            cur.title = line.trim_end_matches(['：', ':']).to_string();
            i += 1;
            continue;
        }
        let has_cjk = !line.is_ascii();
        if has_cjk {
            push_cur(&mut cur, &mut out);
            cur.title = line.trim_end_matches(['：', ':']).to_string();
            i += 1;
            continue;
        }
        if cur.password.is_empty() {
            cur.password = line.to_string();
            i += 1;
            continue;
        }
        if cur.username.is_empty() {
            cur.username = line.to_string();
            i += 1;
            continue;
        }
        push_cur(&mut cur, &mut out);
        cur.title = line.trim_end_matches(['：', ':']).to_string();
        i += 1;
    }
    push_cur(&mut cur, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::parse_blob;

    #[test]
    fn 标签行换行式() {
        let text = "接入地址\nhttps://36.112.20.59:52125\n账号\n潘伟锋\n密码\nTechnical@147258369";
        let r = parse_blob(text);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].url, "https://36.112.20.59:52125");
        assert_eq!(r[0].username, "潘伟锋");
        assert_eq!(r[0].password, "Technical@147258369");
    }

    #[test]
    fn 键值对式() {
        let text = "gitlab-allindata：\nusername: tanweijing\npassword: b3b#H2pD1pAE";
        let r = parse_blob(text);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "gitlab-allindata");
        assert_eq!(r[0].username, "tanweijing");
        assert_eq!(r[0].password, "b3b#H2pD1pAE");
    }

    #[test]
    fn 斜杠对与域名() {
        let text = "sql审核平台：\nsqlauditv2.txffp.com/\ntanweijing / JmPe74Db5jf{";
        let r = parse_blob(text);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "sql审核平台");
        assert_eq!(r[0].url, "https://sqlauditv2.txffp.com/");
        assert_eq!(r[0].username, "tanweijing");
        assert_eq!(r[0].password, "JmPe74Db5jf{");
    }

    #[test]
    fn 手机号与多段() {
        let text = "ruoyi-mos\n18211033965\n\nboms测试账号\n18710905369\n18235356889Yz\n\n测试xxljob\nhttp://172.20.74.14:6060/xxl-job-admin\nxxljob/08uD!3Das1$o";
        let r = parse_blob(text);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].title, "ruoyi-mos");
        assert_eq!(r[0].username, "18211033965");
        assert_eq!(r[1].title, "boms测试账号");
        assert_eq!(r[1].username, "18710905369");
        assert_eq!(r[1].password, "18235356889Yz");
        assert_eq!(r[2].title, "测试xxljob");
        assert_eq!(r[2].url, "http://172.20.74.14:6060/xxl-job-admin");
        assert_eq!(r[2].username, "xxljob");
        assert_eq!(r[2].password, "08uD!3Das1$o");
    }
}
