//! 应用自身设置：zap.db 里的键值表。启动时读默认值，保存后即时生效。

use rusqlite::{Connection, params};

pub const DEFAULT_HOTKEY: &str = "ctrl+alt+space";
pub const DEFAULT_CLIP_MAX: i64 = 500;
/// 密码复制后自动清空剪贴板的秒数，0 = 不清空
pub const DEFAULT_PWD_CLEAR_SECS: i64 = 45;
/// Everything 未运行时是否自动拉起（不自动下载安装）
pub const DEFAULT_EVERYTHING_AUTOSTART: bool = true;

pub const KEY_HOTKEY: &str = "hotkey";
pub const KEY_CLIP_ENABLED: &str = "clip_enabled";
pub const KEY_CLIP_MAX: &str = "clip_max";
pub const KEY_PWD_CLEAR_SECS: &str = "pwd_clear_secs";
pub const KEY_EVERYTHING_AUTOSTART: &str = "everything_autostart";

pub fn init_table(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_settings (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );",
    );
}

pub fn get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM app_settings WHERE key=?1",
        params![key],
        |r| r.get(0),
    )
    .ok()
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO app_settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 便捷读取：解析失败/未设置时回退默认值（测试用）
#[cfg(test)]
pub fn get_or<T: std::str::FromStr>(conn: &Connection, key: &str, default: T) -> T {
    get(conn, key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 键值表读写与默认回退() {
        let conn = Connection::open_in_memory().unwrap();
        init_table(&conn);
        assert_eq!(get(&conn, KEY_HOTKEY), None);
        assert_eq!(get_or(&conn, KEY_CLIP_MAX, DEFAULT_CLIP_MAX), 500);

        set(&conn, KEY_CLIP_MAX, "1000").unwrap();
        assert_eq!(get_or(&conn, KEY_CLIP_MAX, DEFAULT_CLIP_MAX), 1000);
        // 覆盖写
        set(&conn, KEY_CLIP_MAX, "100").unwrap();
        assert_eq!(get(&conn, KEY_CLIP_MAX).as_deref(), Some("100"));
        // 坏值回退默认
        set(&conn, KEY_CLIP_MAX, "abc").unwrap();
        assert_eq!(get_or(&conn, KEY_CLIP_MAX, DEFAULT_CLIP_MAX), 500);
    }
}
