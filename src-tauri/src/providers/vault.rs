//! 账号库：密码经 DPAPI 加密落盘（绑定当前 Windows 账户 + zap 熵），保护域交互。
//! 铁律：密码永不出现在搜索结果、日志或剪贴板历史中。
//! 加解密在 dpapi.rs，智能导入的文本解析在 acctparse.rs

use std::time::Duration;

use rusqlite::{Connection, params};

use super::SearchItem;
use super::dpapi;

#[cfg(windows)]
use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;

// 智能导入的类型与入口经 vault 再导出（lib.rs 命令沿用 vault:: 路径）
pub use super::acctparse::{ParsedAccount, parse_blob};

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
    // 旧库迁移：补 totp_secret 列（DPAPI 密文，与密码同待遇）
    let _ = conn.execute_batch("ALTER TABLE accounts ADD COLUMN totp_enc TEXT NOT NULL DEFAULT ''");
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
///
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
                        super::diag::log(&format!("焚除判定回读失败（第 {attempt}/6 次）：{e}"));
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
    totp: &str,
) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("标题不能为空".into());
    }
    let enc = dpapi::encrypt(password)?;
    let totp_enc = normalize_totp(totp)?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO accounts(title, username, password_enc, url, notes, totp_enc, created_at, updated_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![title, username, enc, url, notes, totp_enc, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// TOTP 入库归一：空串 = 不启用；非空须是可解析的 Base32 / otpauth 链接，
/// 归一为标准大写 Base32 后 DPAPI 加密。无效输入在此报错（保存即校验）
fn normalize_totp(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let secret = super::totp::extract_secret(trimmed)?;
    dpapi::encrypt(&secret)
}

/// 读 TOTP 密钥明文（未配置返回空串）
fn totp_plain(totp_enc: &str) -> Result<String, String> {
    if totp_enc.is_empty() {
        return Ok(String::new());
    }
    dpapi::decrypt(totp_enc)
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

    let q = query.trim().to_lowercase();
    let mut scored: Vec<(f32, i64, i64)> = rows
        .iter()
        .filter_map(|row| {
            // 备注也参与匹配（标题/用户名/网址/备注）
            let hay = format!("{} {} {} {}", row.1, row.2, row.3, row.4);
            super::score_lowered(&hay, &q).map(|s| (s, row.0, row.5))
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
            let plain = dpapi::decrypt(&enc)?;
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
            let plain = dpapi::decrypt(&enc)?;
            Ok(plain)
        }
        "totp" => {
            // 当前验证码（仅展示，不复制）
            let enc: String = conn
                .query_row(
                    "SELECT totp_enc FROM accounts WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let secret = totp_plain(&enc)?;
            if secret.is_empty() {
                return Err("该账号未配置 TOTP".into());
            }
            super::totp::current_code(&secret)
        }
        "totp_copy" => {
            // 复制当前验证码：排除剪贴板历史与云同步；验证码 ≤30s 自过期，不做焚除
            let enc: String = conn
                .query_row(
                    "SELECT totp_enc FROM accounts WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            let secret = totp_plain(&enc)?;
            if secret.is_empty() {
                return Err("该账号未配置 TOTP".into());
            }
            let code = super::totp::current_code(&secret)?;
            super::clipboard::begin_self_copy();
            copy_text_private(&code)?;
            bump(conn);
            Ok("验证码已复制".into())
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

/// 账号完整记录（get 的返回）：title, username, password明文, url, notes, pinned, totp密钥
pub type VaultRecord = (String, String, String, String, String, i64, String);

/// 取单条完整记录（编辑预填用，含解密后的密码明文与 TOTP 密钥）
pub fn get(conn: &Connection, id: i64) -> Result<VaultRecord, String> {
    let row = conn
        .query_row(
            "SELECT title, username, password_enc, url, notes, pinned, totp_enc FROM accounts WHERE id=?1",
            params![id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let plain = dpapi::decrypt(&row.2)?;
    let totp = totp_plain(&row.6)?;
    Ok((row.0, row.1, plain, row.3, row.4, row.5, totp))
}

/// 更新条目：密码字段留空 = 保持不变（其余字段照常覆盖，TOTP 空串 = 清除）
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
    totp: &str,
) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("标题不能为空".into());
    }
    let totp_enc = normalize_totp(totp)?;
    let new_enc = if password.trim().is_empty() {
        None
    } else {
        Some(dpapi::encrypt(password)?)
    };
    match new_enc {
        Some(enc) => conn
            .execute(
                "UPDATE accounts SET title=?1, username=?2, password_enc=?3, url=?4, notes=?5, pinned=?6, totp_enc=?7, updated_at=?8 WHERE id=?9",
                params![title, username.trim(), enc, url.trim(), notes, pinned, totp_enc, now_ms(), id],
            ),
        None => conn
            .execute(
                "UPDATE accounts SET title=?1, username=?2, url=?3, notes=?4, pinned=?5, totp_enc=?6, updated_at=?7 WHERE id=?8",
                params![title, username.trim(), url.trim(), notes, pinned, totp_enc, now_ms(), id],
            ),
    }
    .map_err(|e| e.to_string())?;
    Ok(())
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
        add(&tx, &title, username, &e.password, url, "", "")?;
        n += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(n)
}

// ---------- 备份（加密导出/恢复）----------

/// 读全部账号为备份条目（密码解密为明文——密文由 backup 模块整体加密保护）
pub fn backup_read_all(conn: &Connection) -> Result<Vec<super::backup::BackupEntry>, String> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT title, username, password_enc, url, notes, pinned, totp_enc FROM accounts ORDER BY id",
    ) else {
        return Err("读取账号失败".into());
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, String>(6)?,
        ))
    }) else {
        return Err("读取账号失败".into());
    };
    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(super::backup::BackupEntry {
            title: row.0,
            username: row.1,
            password: dpapi::decrypt(&row.2)?,
            url: row.3,
            notes: row.4,
            pinned: row.5,
            totp: totp_plain(&row.6)?,
        });
    }
    Ok(out)
}

/// 备份条目的有效标题（与 backup_restore 的入库标题同一套兜底规则）：
/// 标题缺省按 用户名 → 网址 → 固定名 递补。重复检测必须用同一口径
fn backup_effective_title(username: &str, url: &str, title: &str) -> String {
    let title = title.trim();
    if !title.is_empty() {
        title.to_string()
    } else if !username.trim().is_empty() {
        username.trim().to_string()
    } else if !url.trim().is_empty() {
        url.trim().to_string()
    } else {
        "未命名账号".to_string()
    }
}

/// 疑似重复检测：有效标题 + 用户名 与现有账号比对（忽略大小写与首尾空白）。
/// 只提示不决策——恢复时跳过与否由用户选
pub fn backup_duplicates(conn: &Connection, entries: &[super::backup::BackupEntry]) -> Vec<bool> {
    entries
        .iter()
        .map(|e| {
            let title = backup_effective_title(&e.username, &e.url, &e.title);
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM accounts
                     WHERE lower(trim(title)) = lower(?1)
                       AND lower(trim(username)) = lower(?2)",
                    params![title, e.username.trim()],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            exists > 0
        })
        .collect()
}

/// 备份恢复：全字段写入（含 pinned），整批一个事务。
/// skip_duplicates 时逐条比对活库（同有效标题+用户名，忽略大小写）跳过疑似重复——
/// 在事务内活查而非快照，备份内部的重复条目也只会写入第一条。
/// 返回 (写入数, 跳过数)
pub fn backup_restore(
    conn: &Connection,
    entries: Vec<super::backup::BackupEntry>,
    skip_duplicates: bool,
) -> Result<(usize, usize), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut n = 0usize;
    let mut skipped = 0usize;
    for e in entries {
        let title = backup_effective_title(&e.username, &e.url, &e.title);
        if skip_duplicates {
            let exists: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM accounts
                     WHERE lower(trim(title)) = lower(?1)
                       AND lower(trim(username)) = lower(?2)",
                    params![title, e.username.trim()],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if exists > 0 {
                skipped += 1;
                continue;
            }
        }
        let enc = dpapi::encrypt(&e.password)?;
        let totp_enc = if e.totp.is_empty() {
            String::new()
        } else {
            dpapi::encrypt(&e.totp)?
        };
        let now = now_ms();
        tx.execute(
            "INSERT INTO accounts(title, username, password_enc, url, notes, created_at, updated_at, pinned, totp_enc)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?8)",
            params![title, e.username.trim(), enc, e.url.trim(), e.notes, now, e.pinned, totp_enc],
        )
        .map_err(|e| e.to_string())?;
        n += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok((n, skipped))
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
        add(&conn, "甲账号", "u1", "p", "", "", "").unwrap();
        // updated_at 精度为毫秒：错开时间保证"新的在前"可判定
        std::thread::sleep(Duration::from_millis(3));
        add(&conn, "乙账号", "u2", "p", "", "", "").unwrap();
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
            "",
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
        add(&conn, "站点", "user", "old-pass", "https://a.com", "", "").unwrap();
        let (_, _, plain, _, _, _, _) = get(&conn, 1).unwrap();
        assert_eq!(plain, "old-pass");

        update(
            &conn,
            1,
            "站点2",
            "user2",
            "",
            "https://b.com",
            "备注",
            0,
            "",
        )
        .unwrap();
        let (t, u, p, url, notes, _, totp) = get(&conn, 1).unwrap();
        assert_eq!(t, "站点2");
        assert_eq!(u, "user2");
        assert_eq!(p, "old-pass", "留空密码应保持原值");
        assert_eq!(url, "https://b.com");
        assert_eq!(notes, "备注");
        assert_eq!(totp, "", "未配置 TOTP 应为空");

        // 传入新密码则覆盖
        update(&conn, 1, "站点2", "user2", "new-pass", "", "", 0, "").unwrap();
        let (_, _, p, _, _, _, _) = get(&conn, 1).unwrap();
        assert_eq!(p, "new-pass");
    }

    #[test]
    fn 置顶条目排最前且带标记() {
        let conn = mem_db();
        add(&conn, "甲账号", "", "", "", "", "").unwrap();
        add(&conn, "乙账号", "", "", "", "", "").unwrap();
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
        add(&conn, "公司邮箱", "me", "pw", "", "公司内部 systems", "").unwrap();
        assert_eq!(search(&conn, "systems").len(), 1);
        assert_eq!(search(&conn, "notes不存在词").len(), 0);
    }

    #[test]
    fn 复制操作更新最近使用() {
        let conn = mem_db();
        add(&conn, "A", "u1", "p1", "", "", "").unwrap();
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

    #[test]
    fn 备份读全量与恢复往返() {
        let conn = mem_db();
        add(&conn, "甲", "u1", "p1", "https://a.com", "备注甲", "").unwrap();
        std::thread::sleep(Duration::from_millis(3));
        add(&conn, "乙", "u2", "p2", "", "", "").unwrap();
        action(&conn, 1, "pin", 0).unwrap();

        // 导出读全量：密码明文、pinned 保留
        let entries = backup_read_all(&conn).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].password, "p1");
        assert_eq!(entries[0].pinned, 1);

        // 恢复到另一个库：全字段还原（含 pinned）
        let conn2 = mem_db();
        let (n, skipped) = backup_restore(&conn2, entries, false).unwrap();
        assert_eq!(n, 2);
        assert_eq!(skipped, 0, "空库恢复无重复可跳");
        let (t, u, p, url, notes, pinned, totp) = get(&conn2, 1).unwrap();
        assert_eq!((t.as_str(), u.as_str(), p.as_str()), ("甲", "u1", "p1"));
        assert_eq!(url, "https://a.com");
        assert_eq!(notes, "备注甲");
        assert_eq!(pinned, 1, "置顶状态应随备份还原");
        assert_eq!(totp, "", "未配置 TOTP 恢复为空");
        let items = search(&conn2, "");
        assert_eq!(items.len(), 2);
        assert!(items[0].title.starts_with("📌"), "恢复后置顶仍排最前");
    }

    #[test]
    fn totp配置出码与受保护复制() {
        let conn = mem_db();
        // otpauth 链接保存即归一为标准 Base32
        add(
            &conn,
            "有OTP",
            "u",
            "p",
            "",
            "",
            "otpauth://totp/x?secret=gezd gnbv gy3t qojq",
        )
        .unwrap();
        let (_, _, _, _, _, _, totp) = get(&conn, 1).unwrap();
        assert_eq!(totp, "GEZDGNBVGY3TQOJQ", "应去空白并大写化归一存储");
        // 当前码为合法 6 位且可受保护复制
        let code = action(&conn, 1, "totp", 0).unwrap();
        assert_eq!(code.len(), 6, "应为 6 位验证码");
        assert!(code.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(
            action(&conn, 1, "totp_copy", 0).unwrap(),
            "验证码已复制",
            "验证码复制应抑制剪贴板历史"
        );
        // 非法密钥保存即报错；未配置账号取码报错
        assert!(add(&conn, "坏OTP", "u", "p", "", "", "not-a-key!!").is_err());
        add(&conn, "无OTP", "u", "p", "", "", "").unwrap();
        assert_eq!(
            action(&conn, 2, "totp", 0).unwrap_err(),
            "该账号未配置 TOTP"
        );
    }

    #[test]
    fn 重复检测与跳过恢复() {
        let conn = mem_db();
        add(&conn, "甲", "u1", "p1", "", "", "").unwrap();
        add(&conn, "gitlab", "U2", "p2", "", "", "").unwrap();

        let mk = |title: &str, username: &str| crate::providers::backup::BackupEntry {
            title: title.into(),
            username: username.into(),
            password: "pw".into(),
            url: String::new(),
            notes: String::new(),
            pinned: 0,
            totp: String::new(),
        };
        // 同标题（大小写不同）+ 同用户名（大小写不同）= 疑似重复；批内两条同键
        let entries = vec![
            mk("甲", "u1"),
            mk("GitLab", "u2"),
            mk("新站", "u9"),
            mk("批内", "ub"),
            mk("批内", "ub"),
        ];
        let dups = backup_duplicates(&conn, &entries);
        assert_eq!(dups, vec![true, true, false, false, false]);

        // 跳过模式：命中现有库的 2 条跳过；批内第二条被活库检查拦下（只写第一条）
        let (written, skipped) = backup_restore(&conn, entries.clone(), true).unwrap();
        assert_eq!((written, skipped), (2, 3), "新站+批内第一条入库");
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 4, "原 2 + 写入 2，批内重复未翻倍");

        // 不跳过模式：全量追加（原测试口径）
        let (n, skipped) = backup_restore(&conn, entries, false).unwrap();
        assert_eq!((n, skipped), (5, 0));
    }
}
