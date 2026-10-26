//! 应用自身设置：zap.db 里的键值表。启动时读默认值，保存后即时生效。

use rusqlite::{Connection, params};

pub const DEFAULT_HOTKEY: &str = "ctrl+alt+space";
pub const DEFAULT_CLIP_MAX: i64 = 500;
/// 剪贴板条目选中后是否自动粘贴到原窗口（0 = 关闭，仅回填）
pub const DEFAULT_CLIP_AUTOPASTE: bool = true;
/// 密码复制后自动清空剪贴板的秒数，0 = 不清空
pub const DEFAULT_PWD_CLEAR_SECS: i64 = 45;
/// Everything 未运行时是否自动拉起（不自动下载安装）
pub const DEFAULT_EVERYTHING_AUTOSTART: bool = true;

pub const KEY_HOTKEY: &str = "hotkey";
pub const KEY_CLIP_ENABLED: &str = "clip_enabled";
pub const KEY_CLIP_MAX: &str = "clip_max";
pub const KEY_CLIP_AUTOPASTE: &str = "clip_autopaste";
pub const KEY_PWD_CLEAR_SECS: &str = "pwd_clear_secs";
pub const KEY_EVERYTHING_AUTOSTART: &str = "everything_autostart";
/// 外观模式（dark/light/system），非法值前端按 dark 回退
pub const KEY_APPEARANCE: &str = "appearance";
/// 透明面板（亚克力窗口特效），系统不支持时面板保持不透明
pub const KEY_PANEL_TRANSPARENCY: &str = "panel_transparency";
pub const DEFAULT_PANEL_TRANSPARENCY: bool = true;
/// 账号库直达热键（空串 = 未绑定）：呼出面板并进入 pw 保护域
pub const KEY_VAULT_HOTKEY: &str = "vault_hotkey";
/// 快速粘贴直达热键（空串 = 未绑定）：最近一条剪贴板直接粘贴到前台窗口
pub const KEY_CLIP_HOTKEY: &str = "clip_hotkey";
/// 超级面板直达热键（空串 = 未绑定）：抓取选中文本出动作菜单。
/// 早版用长按右键手势，因与各应用右键菜单冲突风险大改为热键触发
pub const KEY_SUPERPANEL_HOTKEY: &str = "superpanel_hotkey";

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
