//! 账号库动作分发（P3.2）：从 vault.rs 的巨型 match 拆出的动作函数表。
//! 每个动作一个私有函数，顶层 action() 只做 dispatch；行为与拆分前逐分支等价，
//! 尤其保留 begin_self_copy → copy_text_private → bump 的顺序、密码延时清空
//! 的序列号比对，以及 delete 的双缓存失效顺序。对外仍由 vault::action 导出。

use rusqlite::{Connection, params};

use super::*;
use crate::providers::{clipboard, totp};

/// 复制类动作刷「最近使用」排序（未置顶条目自然前移）
fn bump(conn: &Connection, id: i64) {
    let _ = conn.execute(
        "UPDATE accounts SET updated_at=?1 WHERE id=?2",
        params![now_ms(), id],
    );
}

/// 可复制字段：SQL 与文案都由内部枚举给出，不接收外部字符串，零动态 SQL。
#[derive(Clone, Copy)]
enum CopyColumn {
    Username,
    Url,
    Notes,
}

impl CopyColumn {
    fn sql(self) -> &'static str {
        match self {
            Self::Username => "SELECT username FROM accounts WHERE id=?1",
            Self::Url => "SELECT url FROM accounts WHERE id=?1",
            Self::Notes => "SELECT notes FROM accounts WHERE id=?1",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Username => "用户名",
            Self::Url => "网址",
            Self::Notes => "备注",
        }
    }
}

/// username / url_copy / notes 三分支的同构实现：
/// 查列 → 抑制剪贴板历史 → 受保护复制 → 刷最近使用 → 反馈。
/// 用户名与密码成对泄露才是完整凭证（评审 P1-13），备注可能含敏感信息，
/// 三者都走与密码相同的受保护通道。
fn copy_column(conn: &Connection, id: i64, col: CopyColumn) -> Result<String, String> {
    let value: String = conn
        .query_row(col.sql(), params![id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    clipboard::begin_self_copy();
    copy_text_private(&value)?;
    bump(conn, id);
    Ok(format!("{}已复制", col.label()))
}

/// 密码复制反馈文案：抽出成纯函数，便于单测锁住 0 / >0 两条分叉。
fn password_feedback(pwd_clear_secs: i64) -> String {
    if pwd_clear_secs > 0 {
        format!("密码已复制，{pwd_clear_secs} 秒后自动清除")
    } else {
        "密码已复制".into()
    }
}

/// 密码复制：DPAPI 解密 → 受保护复制 → 延时清空。
/// 复制完成后立刻取序列号，清空时序列号未变才清（不覆盖用户的新复制）。
fn copy_password(conn: &Connection, id: i64, pwd_clear_secs: i64) -> Result<String, String> {
    let enc: String = conn
        .query_row(
            "SELECT password_enc FROM accounts WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let plain = dpapi::decrypt(&enc)?;
    clipboard::begin_self_copy();
    copy_text_private(&plain)?;
    bump(conn, id);
    schedule_clear(plain, clipboard_seq(), pwd_clear_secs);
    Ok(password_feedback(pwd_clear_secs))
}

/// 显式查看：返回解密后的密码明文（前端只做一次性展示）
fn reveal_password(conn: &Connection, id: i64) -> Result<String, String> {
    let enc: String = conn
        .query_row(
            "SELECT password_enc FROM accounts WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    dpapi::decrypt(&enc)
}

/// 返回网址本身：前端「打开网址」流拿它作 open_item 的 target
///（与 "url_copy" 是两个消费方，2026-11-04 P1 拆分）
fn read_url(conn: &Connection, id: i64) -> Result<String, String> {
    let url: String = conn
        .query_row("SELECT url FROM accounts WHERE id=?1", params![id], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    bump(conn, id);
    Ok(url)
}

/// 当前验证码（展示与复制共用）：未配置 TOTP 时统一报错
fn current_totp(conn: &Connection, id: i64) -> Result<String, String> {
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
    totp::current_code(&secret)
}

/// 复制当前验证码：排除剪贴板历史与云同步；验证码 ≤30s 自过期，不做延时清空
fn copy_totp(conn: &Connection, id: i64) -> Result<String, String> {
    let code = current_totp(conn, id)?;
    clipboard::begin_self_copy();
    copy_text_private(&code)?;
    bump(conn, id);
    Ok("验证码已复制".into())
}

fn toggle_pin(conn: &Connection, id: i64) -> Result<String, String> {
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

fn delete_account(conn: &Connection, id: i64) -> Result<String, String> {
    conn.execute("DELETE FROM accounts WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    // 密码集必须同步失效（2026-11-04 P1）：只失效形态缓存会让已删
    // 密码继续抑制剪贴板入库——从历史翻不出、入库又静默失败，双向都错
    invalidate_forms_cache();
    invalidate_password_cache();
    Ok("已删除".into())
}

/// 动作分发。复制动作统一抑制剪贴板历史；密码只在显式查看时返回明文。
pub fn action(
    conn: &Connection,
    id: i64,
    action: &str,
    pwd_clear_secs: i64,
) -> Result<String, String> {
    match action {
        "username" => copy_column(conn, id, CopyColumn::Username),
        "password" => copy_password(conn, id, pwd_clear_secs),
        "url" => read_url(conn, id),
        "url_copy" => copy_column(conn, id, CopyColumn::Url),
        "notes" => copy_column(conn, id, CopyColumn::Notes),
        "reveal" => reveal_password(conn, id),
        "totp" => current_totp(conn, id),
        "totp_copy" => copy_totp(conn, id),
        "pin" => toggle_pin(conn, id),
        "delete" => delete_account(conn, id),
        _ => Err("未知操作".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P3.2 回归锁：抽公共函数后，SQL 与文案的配对不能错位
    #[test]
    fn 复制字段查询与文案一一对应() {
        let cases = [
            (CopyColumn::Username, "username", "用户名"),
            (CopyColumn::Url, "url", "网址"),
            (CopyColumn::Notes, "notes", "备注"),
        ];
        for (col, column, label) in cases {
            assert_eq!(
                col.sql(),
                format!("SELECT {column} FROM accounts WHERE id=?1").as_str()
            );
            assert_eq!(format!("{}已复制", col.label()), format!("{label}已复制"));
        }
    }

    #[test]
    fn 密码复制文案按清空秒数分叉() {
        assert_eq!(password_feedback(0), "密码已复制");
        assert_eq!(password_feedback(45), "密码已复制，45 秒后自动清除");
    }

    /// P3.2 回归锁：10 个动作都有分派（不建表，已知动作应报库错误而不是
    /// 「未知操作」），未知动作仍被拒。
    #[test]
    fn 动作分派不漏分支且未知动作被拒() {
        let conn = Connection::open_in_memory().unwrap();
        for a in [
            "username",
            "password",
            "url",
            "url_copy",
            "notes",
            "reveal",
            "totp",
            "totp_copy",
            "pin",
            "delete",
        ] {
            let err = action(&conn, 1, a, 0).unwrap_err();
            assert_ne!(err, "未知操作", "动作 {a} 漏了分派");
        }
        assert_eq!(action(&conn, 1, "nope", 0).unwrap_err(), "未知操作");
    }
}
