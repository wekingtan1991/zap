//! 账号库：密码经 DPAPI 加密落盘（绑定当前 Windows 账户 + zap 熵），保护域交互。
//! 铁律：密码永不出现在搜索结果、日志或剪贴板历史中。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use rusqlite::{params, Connection};

use super::SearchItem;

/// 账号库复制密码时置位：剪贴板监听跳过下一次入库（阅后即焚）
pub static SUPPRESS_NEXT_CLIP: AtomicBool = AtomicBool::new(false);

/// 应用专属熵：让 DPAPI 密文只有 zap 能解，同账户下其他程序无能为力
const ENTROPY: &[u8] = b"zap:vault:v1";

/// 智能导入的单条解析结果（可编辑预览后批量入库）
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParsedAccount {
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
}

pub fn init_table(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS accounts (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             title TEXT NOT NULL,
             username TEXT NOT NULL DEFAULT '',
             password_enc TEXT NOT NULL DEFAULT '',
             url TEXT NOT NULL DEFAULT '',
             notes TEXT NOT NULL DEFAULT '',
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL
         );",
    );
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------- DPAPI ----------

#[cfg(windows)]
fn dpapi_encrypt(plain: &str) -> Result<String, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};

    let plain_bytes = plain.as_bytes();
    let blob_in = CRYPT_INTEGER_BLOB {
        cbData: plain_bytes.len() as u32,
        pbData: plain_bytes.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let out = {
        let mut blob_out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        unsafe {
            CryptProtectData(
                &blob_in,
                PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                0,
                &mut blob_out,
            )
        }
        .map_err(|e| format!("DPAPI 加密失败: {e}"))?;
        blob_out
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(unsafe {
        std::slice::from_raw_parts(out.pbData, out.cbData as usize)
    });
    unsafe {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
    }
    Ok(encoded)
}

#[cfg(windows)]
fn dpapi_decrypt(encoded: &str) -> Result<String, String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| e.to_string())?;
    let blob_in = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let out = {
        let mut blob_out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        unsafe {
            CryptUnprotectData(
                &blob_in,
                None,
                Some(&entropy),
                None,
                None,
                0,
                &mut blob_out,
            )
        }
        .map_err(|e| format!("DPAPI 解密失败: {e}"))?;
        blob_out
    };
    let plain = String::from_utf8_lossy(unsafe {
        std::slice::from_raw_parts(out.pbData, out.cbData as usize)
    })
    .to_string();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
    }
    Ok(plain)
}

#[cfg(not(windows))]
fn dpapi_encrypt(_: &str) -> Result<String, String> {
    Err("仅支持 Windows".into())
}

#[cfg(not(windows))]
fn dpapi_decrypt(_: &str) -> Result<String, String> {
    Err("仅支持 Windows".into())
}

// ---------- 剪贴板 ----------

fn copy_text(text: &str) -> Result<(), String> {
    arboard::Clipboard::new()
        .map_err(|e| e.to_string())?
        .set_text(text.to_string())
        .map_err(|e| e.to_string())
}

/// 阅后即焚：延时后若剪贴板仍是这段密码则自动清除（不覆盖用户的新复制）；secs<=0 关闭
fn schedule_clear(password: String, secs: i64) {
    if secs <= 0 {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(secs as u64));
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if cb.get_text().map(|t| t == password).unwrap_or(false) {
                let _ = cb.clear();
            }
        }
    });
}

// ---------- 数据操作 ----------

pub fn add(
    conn: &Connection,
    title: &str,
    username: &str,
    password: &str,
    url: &str,
    notes: &str,
) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("标题不能为空".into());
    }
    let enc = dpapi_encrypt(password)?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO accounts(title, username, password_enc, url, notes, created_at, updated_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![title, username, enc, url, notes, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn search(conn: &Connection, query: &str) -> Vec<SearchItem> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, title, username, url FROM accounts ORDER BY updated_at DESC LIMIT 200",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    }) else {
        return Vec::new();
    };
    let rows: Vec<(i64, String, String, String)> = rows.flatten().collect();
    let mut scored: Vec<(f32, i64)> = rows
        .iter()
        .filter_map(|row| {
            let hay = format!("{} {} {}", row.1, row.2, row.3);
            super::score_match(&hay, query).map(|s| (s, row.0))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(b.1.cmp(&a.1)));
    scored.truncate(8);
    scored
        .into_iter()
        .enumerate()
        .filter_map(|(i, (_, id))| {
            let (_, title, username, url) = rows.iter().find(|r| r.0 == id)?;
            let subtitle = if url.is_empty() {
                username.clone()
            } else {
                format!("{username} · {url}")
            };
            Some(SearchItem {
                id: format!("vault:{id}"),
                title: title.clone(),
                subtitle,
                kind: "vault".into(),
                target: id.to_string(),
                icon: None,
                score: 70.0 - i as f32,
            })
        })
        .collect()
}

/// 动作分发。密码只复制、永不返回明文。
pub fn action(conn: &Connection, id: i64, action: &str, pwd_clear_secs: i64) -> Result<String, String> {
    match action {
        "username" => {
            let username: String = conn
                .query_row(
                    "SELECT username FROM accounts WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            copy_text(&username)?;
            Ok("用户名已复制".into())
        }
        "password" => {
            let enc: String = conn
                .query_row(
                    "SELECT password_enc FROM accounts WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let plain = dpapi_decrypt(&enc)?;
            SUPPRESS_NEXT_CLIP.store(true, Ordering::Relaxed);
            copy_text(&plain)?;
            schedule_clear(plain, pwd_clear_secs);
            Ok(if pwd_clear_secs > 0 {
                format!("密码已复制，{pwd_clear_secs} 秒后自动清除")
            } else {
                "密码已复制".into()
            })
        }
        "url" => {
            let url: String = conn
                .query_row("SELECT url FROM accounts WHERE id=?1", params![id], |r| {
                    r.get(0)
                })
                .map_err(|e| e.to_string())?;
            Ok(url)
        }
        "delete" => {
            conn.execute("DELETE FROM accounts WHERE id=?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok("已删除".into())
        }
        _ => Err("未知操作".into()),
    }
}

// ---------- 智能导入：文本解析 ----------

const LABEL_USER: &[&str] = &[
    "账号", "用户名", "用户", "登录名", "账号名", "user", "username", "account", "login",
];
const LABEL_PASS: &[&str] = &["密码", "口令", "password", "pwd", "pass"];
const LABEL_URL: &[&str] = &["接入地址", "地址", "链接", "网址", "url", "site", "link"];

fn label_field(line_lower: &str) -> Option<&'static str> {
    if LABEL_URL.iter().any(|k| line_lower == *k) {
        return Some("url");
    }
    if LABEL_USER.iter().any(|k| line_lower == *k) {
        return Some("user");
    }
    if LABEL_PASS.iter().any(|k| line_lower == *k) {
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
        && host.split('.').all(|p| !p.is_empty() && p.parse::<u8>().is_ok());
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
    line.len() == 11
        && line.starts_with('1')
        && line.chars().all(|c| c.is_ascii_digit())
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
        "url" => {
            if e.url.is_empty() {
                e.url = ensure_scheme(val);
            }
        }
        _ => {}
    }
}

fn has_any_cred(e: &ParsedAccount) -> bool {
    !e.username.is_empty() || !e.password.is_empty()
}

/// 多格式启发式解析。识别不了的细节交给可编辑预览修正。
pub fn parse_blob(text: &str) -> Vec<ParsedAccount> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
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
                Some(next) if split_kv(next).is_none() && label_field(&next.to_lowercase()).is_none() => {
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
        let has_cjk = line.chars().any(|c| !c.is_ascii());
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

/// 批量导入，返回成功条数
pub fn import(conn: &Connection, entries: Vec<ParsedAccount>) -> Result<usize, String> {
    let mut n = 0usize;
    for e in entries {
        let title = e.title.trim();
        if title.is_empty() && e.username.is_empty() && e.password.is_empty() {
            continue;
        }
        let title = if title.is_empty() { e.username.clone() } else { e.title.trim().to_string() };
        add(conn, &title, &e.username, &e.password, &e.url, "")?;
        n += 1;
    }
    Ok(n)
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
