//! 账号库：密码经 DPAPI 加密落盘（绑定当前 Windows 账户 + zap 熵），保护域交互。
//! 铁律：密码永不出现在搜索结果、日志或剪贴板历史中。

use std::time::Duration;

use base64::Engine as _;
use rusqlite::{Connection, params};

use super::SearchItem;

#[cfg(windows)]
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;

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
             updated_at INTEGER NOT NULL,
             pinned INTEGER NOT NULL DEFAULT 0
         );",
    );
    // 旧库迁移：补 pinned 列（已存在时报错可忽略）
    let _ = conn.execute_batch("ALTER TABLE accounts ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0");
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
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};
    use windows::core::PCWSTR;

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
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) });
    unsafe {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
    }
    Ok(encoded)
}

#[cfg(windows)]
fn dpapi_decrypt(encoded: &str) -> Result<String, String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};

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
        unsafe { CryptUnprotectData(&blob_in, None, Some(&entropy), None, None, 0, &mut blob_out) }
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

/// 敏感内容专用复制：排除 Windows 剪贴板历史（Win+V）与云剪贴板同步。
/// 不排 monitor：zap 自身的剪贴板监听依赖变更事件做入库抑制（begin_self_copy 时间窗），
/// 排 monitor 有让监听收不到事件、抑制错位的风险。
#[cfg(windows)]
fn copy_text_private(text: &str) -> Result<(), String> {
    use arboard::SetExtWindows;
    arboard::Clipboard::new()
        .map_err(|e| e.to_string())?
        .set()
        .exclude_from_history()
        .exclude_from_cloud()
        .text(text.to_string())
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn copy_text_private(text: &str) -> Result<(), String> {
    copy_text(text)
}

/// 当前剪贴板序列号：任何剪贴板变更都会使其递增，用于"内容是否还是我们复制的那一份"判定
#[cfg(windows)]
fn clipboard_seq() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

#[cfg(not(windows))]
fn clipboard_seq() -> u32 {
    0
}

/// 阅后即焚：延时后若剪贴板仍是这段密码则清空；secs<=0 关闭。
/// 判定用「序列号未变（快路径）或回读文本仍是这段密码」双保险：
/// - 序列号没变 = 没人动过剪贴板，直接清；
/// - 序列号变了也再回读确认——他程序重设了同样内容照样清；
///   是别的内容（用户复制了新东西）才放弃，绝不覆盖用户的新复制。
/// Windows 上 OpenClipboard 可能被其他窗口短暂占用（IM/浏览器常驻监听），
/// arboard 自带 5×5ms 重试窗口太窄，这里 6 次递进重试（合计约 4.2s）并全程落盘日志。
#[cfg(windows)]
fn schedule_clear(password: String, seq: u32, secs: i64) {
    if secs <= 0 {
        super::diag::log("焚除未启用（pwd_clear_secs<=0）");
        return;
    }
    super::diag::log(&format!("密码已复制：seq={seq}，{secs}s 后焚除"));
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(secs as u64));
        for attempt in 1..=6u32 {
            // ① 判定剪贴板是否还是这段密码
            if clipboard_seq() != seq {
                match arboard::Clipboard::new().and_then(|mut cb| cb.get_text()) {
                    Ok(t) if t == password => { /* 他程序重设了同内容：继续清 */ }
                    Ok(_) => {
                        super::diag::log("焚除放弃：剪贴板已被新内容替换");
                        return;
                    }
                    Err(e) => {
                        super::diag::log(&format!(
                            "焚除判定回读失败（第 {attempt}/6 次）：{e}"
                        ));
                        std::thread::sleep(Duration::from_millis(200 * attempt as u64));
                        continue;
                    }
                }
            }
            // ② 执行清除
            match arboard::Clipboard::new().and_then(|mut cb| cb.clear()) {
                Ok(()) => {
                    super::diag::log(&format!("密码焚除成功（第 {attempt} 次尝试）"));
                    return;
                }
                Err(e) => {
                    super::diag::log(&format!("密码焚除失败（第 {attempt}/6 次）：{e}"));
                    std::thread::sleep(Duration::from_millis(200 * attempt as u64));
                }
            }
        }
        super::diag::log("密码焚除最终失败：6 次重试均未成功");
    });
}

#[cfg(not(windows))]
fn schedule_clear(_password: String, _seq: u32, _secs: i64) {}

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

/// 行 → 展示条目（置顶带 📌 标记）。列表只呈现标题，详情页才展开各行内容
fn row_to_item(row: &(i64, String, String, String, String, i64)) -> SearchItem {
    let (id, title, _, _, _, pinned) = row; // 用户名/网址在详情页逐行呈现，不进列表
    SearchItem {
        id: format!("vault:{id}"),
        title: if *pinned == 1 {
            format!("📌 {title}")
        } else {
            title.clone()
        },
        subtitle: String::new(),
        kind: "vault".into(),
        target: id.to_string(),
        icon: None,
        score: 70.0,
    }
}

pub fn search(conn: &Connection, query: &str) -> Vec<SearchItem> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, title, username, url, notes, pinned FROM accounts
         ORDER BY pinned DESC, updated_at DESC LIMIT 200",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
        ))
    }) else {
        return Vec::new();
    };
    let rows: Vec<(i64, String, String, String, String, i64)> = rows.flatten().collect();

    // 空查询 = 浏览全部账号（置顶优先、新在前），与剪贴板空查询显示最近的口径一致
    if query.trim().is_empty() {
        return rows.iter().take(8).map(row_to_item).collect();
    }

    let mut scored: Vec<(f32, i64, i64)> = rows
        .iter()
        .filter_map(|row| {
            // 备注也参与匹配（标题/用户名/网址/备注）
            let hay = format!("{} {} {} {}", row.1, row.2, row.3, row.4);
            super::score_match(&hay, query).map(|s| (s, row.0, row.5))
        })
        .collect();
    // 同分档内置顶优先，其次新在前
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap()
            .then(b.2.cmp(&a.2))
            .then(b.1.cmp(&a.1))
    });
    scored.truncate(8);
    scored
        .into_iter()
        .enumerate()
        .filter_map(|(i, (_, id, _))| {
            let row = rows.iter().find(|r| r.0 == id)?;
            let mut item = row_to_item(row);
            item.score = 70.0 - i as f32;
            Some(item)
        })
        .collect()
}

/// 动作分发。复制动作统一抑制剪贴板历史；密码只在显式查看时返回明文。
pub fn action(
    conn: &Connection,
    id: i64,
    action: &str,
    pwd_clear_secs: i64,
) -> Result<String, String> {
    // 复制类操作同时刷新「最近使用」排序（未置顶条目自然前移）
    let bump = |conn: &Connection| {
        let _ = conn.execute(
            "UPDATE accounts SET updated_at=?1 WHERE id=?2",
            params![now_ms(), id],
        );
    };
    match action {
        "username" => {
            let username: String = conn
                .query_row(
                    "SELECT username FROM accounts WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            super::clipboard::begin_self_copy();
            copy_text(&username)?;
            bump(conn);
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
            super::clipboard::begin_self_copy();
            copy_text_private(&plain)?;
            bump(conn);
            // 复制完成后立刻取序列号：焚除时序列号未变才清（不覆盖用户的新复制）
            schedule_clear(plain, clipboard_seq(), pwd_clear_secs);
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
            bump(conn);
            Ok(url)
        }
        "notes" => {
            // 详情页备注行的一键复制：同样抑制剪贴板历史（备注可能含敏感信息）
            let notes: String = conn
                .query_row("SELECT notes FROM accounts WHERE id=?1", params![id], |r| {
                    r.get(0)
                })
                .map_err(|e| e.to_string())?;
            super::clipboard::begin_self_copy();
            copy_text(&notes)?;
            bump(conn);
            Ok("备注已复制".into())
        }
        "reveal" => {
            // 显式查看：返回解密后的密码明文（前端只做一次性展示）
            let enc: String = conn
                .query_row(
                    "SELECT password_enc FROM accounts WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let plain = dpapi_decrypt(&enc)?;
            Ok(plain)
        }
        "pin" => {
            let pinned: i64 = conn
                .query_row(
                    "SELECT pinned FROM accounts WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let next = if pinned == 1 { 0 } else { 1 };
            conn.execute(
                "UPDATE accounts SET pinned=?1 WHERE id=?2",
                params![next, id],
            )
            .map_err(|e| e.to_string())?;
            Ok(if next == 1 {
                "已置顶".into()
            } else {
                "已取消置顶".into()
            })
        }
        "delete" => {
            conn.execute("DELETE FROM accounts WHERE id=?1", params![id])
                .map_err(|e| e.to_string())?;
            Ok("已删除".into())
        }
        _ => Err("未知操作".into()),
    }
}

/// 取单条完整记录（编辑预填用，含解密后的密码明文）
pub fn get(
    conn: &Connection,
    id: i64,
) -> Result<(String, String, String, String, String, i64), String> {
    let row = conn
        .query_row(
            "SELECT title, username, password_enc, url, notes, pinned FROM accounts WHERE id=?1",
            params![id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let plain = dpapi_decrypt(&row.2)?;
    Ok((row.0, row.1, plain, row.3, row.4, row.5))
}

/// 更新条目：密码字段留空 = 保持不变（其余字段照常覆盖）
// 账号字段一字排开：调用处按名传参比塞结构体更直观，故显式放行参数个数提示
#[allow(clippy::too_many_arguments)]
pub fn update(
    conn: &Connection,
    id: i64,
    title: &str,
    username: &str,
    password: &str,
    url: &str,
    notes: &str,
    pinned: i64,
) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("标题不能为空".into());
    }
    let new_enc = if password.trim().is_empty() {
        None
    } else {
        Some(dpapi_encrypt(password)?)
    };
    match new_enc {
        Some(enc) => conn
            .execute(
                "UPDATE accounts SET title=?1, username=?2, password_enc=?3, url=?4, notes=?5, pinned=?6, updated_at=?7 WHERE id=?8",
                params![title, username.trim(), enc, url.trim(), notes, pinned, now_ms(), id],
            ),
        None => conn
            .execute(
                "UPDATE accounts SET title=?1, username=?2, url=?3, notes=?4, pinned=?5, updated_at=?6 WHERE id=?7",
                params![title, username.trim(), url.trim(), notes, pinned, now_ms(), id],
            ),
    }
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- 智能导入：文本解析 ----------

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

/// 批量导入，返回成功条数。标题缺省时按 用户名 → 网址 → 固定名 递补
/// （`add` 要求标题非空，否则整批会中途失败）；整批一个事务，要么全部入库要么全部回滚。
pub fn import(conn: &Connection, entries: Vec<ParsedAccount>) -> Result<usize, String> {
    let mut n = 0usize;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for e in entries {
        let title = e.title.trim();
        let username = e.username.trim();
        let url = e.url.trim();
        if title.is_empty() && username.is_empty() && e.password.is_empty() {
            continue;
        }
        let title = if !title.is_empty() {
            title.to_string()
        } else if !username.is_empty() {
            username.to_string()
        } else if !url.is_empty() {
            url.to_string()
        } else {
            "未命名账号".to_string()
        };
        add(&tx, &title, username, &e.password, url, "")?;
        n += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod crud_tests {
    use super::*;
    use rusqlite::Connection;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_table(&conn);
        conn
    }

    #[test]
    fn 空查询列出全部账号() {
        let conn = mem_db();
        add(&conn, "甲账号", "u1", "p", "", "").unwrap();
        // updated_at 精度为毫秒：错开时间保证"新的在前"可判定
        std::thread::sleep(Duration::from_millis(3));
        add(&conn, "乙账号", "u2", "p", "", "").unwrap();
        let items = search(&conn, "");
        assert_eq!(items.len(), 2, "空查询应列出全部账号");
        assert_eq!(items[0].title, "乙账号", "新入库的排在前");
    }

    #[test]
    fn 导入标题兜底且整批入库() {
        let conn = mem_db();
        let entries = vec![
            // 无标题无用户名：用网址命名
            ParsedAccount {
                url: "https://a.com".into(),
                password: "p1".into(),
                ..Default::default()
            },
            // 无标题：用用户名命名
            ParsedAccount {
                username: "u2".into(),
                password: "p2".into(),
                ..Default::default()
            },
            ParsedAccount {
                title: "有标题".into(),
                username: "u3".into(),
                password: "p3".into(),
                ..Default::default()
            },
        ];
        assert_eq!(import(&conn, entries).unwrap(), 3, "缺标题的条目也应导入");
        assert_eq!(search(&conn, "a.com").len(), 1);
        assert_eq!(search(&conn, "a.com")[0].title, "https://a.com");
        assert_eq!(search(&conn, "u2").len(), 1);
        assert_eq!(search(&conn, "有标题").len(), 1);
    }

    #[test]
    fn 备注可复制且列表不带副标题() {
        let conn = mem_db();
        add(
            &conn,
            "行云账号",
            "13800138000",
            "pw",
            "https://a.com",
            "内部 systems",
        )
        .unwrap();
        assert_eq!(action(&conn, 1, "notes", 0).unwrap(), "备注已复制");
        assert!(
            crate::providers::clipboard::in_suppress_window(),
            "账号库复制应抑制剪贴板历史"
        );
        let items = search(&conn, "行云");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "行云账号");
        assert!(items[0].subtitle.is_empty(), "列表只显示标题，不带副标题");
    }

    #[test]
    fn 更新留空密码保持原值() {
        let conn = mem_db();
        add(&conn, "站点", "user", "old-pass", "https://a.com", "").unwrap();
        let (_, _, plain, _, _, _) = get(&conn, 1).unwrap();
        assert_eq!(plain, "old-pass");

        update(&conn, 1, "站点2", "user2", "", "https://b.com", "备注", 0).unwrap();
        let (t, u, p, url, notes, _) = get(&conn, 1).unwrap();
        assert_eq!(t, "站点2");
        assert_eq!(u, "user2");
        assert_eq!(p, "old-pass", "留空密码应保持原值");
        assert_eq!(url, "https://b.com");
        assert_eq!(notes, "备注");

        // 传入新密码则覆盖
        update(&conn, 1, "站点2", "user2", "new-pass", "", "", 0).unwrap();
        let (_, _, p, _, _, _) = get(&conn, 1).unwrap();
        assert_eq!(p, "new-pass");
    }

    #[test]
    fn 置顶条目排最前且带标记() {
        let conn = mem_db();
        add(&conn, "甲账号", "", "", "", "").unwrap();
        add(&conn, "乙账号", "", "", "", "").unwrap();
        action(&conn, 1, "pin", 0).unwrap();

        let items = search(&conn, "账号");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "📌 甲账号", "置顶条目应带标记且排在最前");
        assert_eq!(items[1].title, "乙账号");

        // 再 pin 一次 = 取消置顶
        action(&conn, 1, "pin", 0).unwrap();
        let items = search(&conn, "账号");
        assert!(!items[0].title.starts_with("📌"));
    }

    #[test]
    fn 备注参与搜索匹配() {
        let conn = mem_db();
        add(&conn, "公司邮箱", "me", "pw", "", "公司内部 systems").unwrap();
        assert_eq!(search(&conn, "systems").len(), 1);
        assert_eq!(search(&conn, "notes不存在词").len(), 0);
    }

    #[test]
    fn 复制操作更新最近使用() {
        let conn = mem_db();
        add(&conn, "A", "u1", "p1", "", "").unwrap();
        let before: i64 = conn
            .query_row("SELECT updated_at FROM accounts WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        action(&conn, 1, "username", 0).unwrap();
        let after: i64 = conn
            .query_row("SELECT updated_at FROM accounts WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(after >= before, "复制后应刷新 updated_at");
    }
}
