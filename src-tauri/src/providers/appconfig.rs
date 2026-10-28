//! 应用自身设置：zap.db 里的键值表。启动时读默认值，保存后即时生效。

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

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
/// 中转站直达热键（空串 = 未绑定）：呼出面板并进入中转站视图
pub const KEY_STAGE_HOTKEY: &str = "stage_hotkey";
/// 全局拖拽落区条开关：任何应用拖文件时在停靠角弹「拖到这，放入中转站」
pub const KEY_STAGEDROP_ENABLED: &str = "stagedrop_enabled";
pub const DEFAULT_STAGEDROP_ENABLED: bool = true;
/// 拖文件到落区条松手后是否自动呼出中转站面板（默认关：面板呼出属行为
/// 变更，交用户显式开启；关闭时放入结果仍由条内反馈呈现）
pub const KEY_STAGEDROP_OPEN_PANEL: &str = "stagedrop_open_panel";
pub const DEFAULT_STAGEDROP_OPEN_PANEL: bool = false;
/// 落区条停靠角（bottom_right/bottom_left/top_right/top_left）：相对拖拽
/// 发生时光标所在显示器的工作区，非法值按默认右下处理
pub const KEY_STAGEDROP_POSITION: &str = "stagedrop_position";
pub const DEFAULT_STAGEDROP_POSITION: &str = "bottom_right";
/// 中转站未固定条目容量上限
pub const KEY_STAGE_CAP: &str = "stage_cap";
pub const DEFAULT_STAGE_CAP: i64 = 50;

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

// ---------- 配置文件（导出/导入） ----------

/// 配置文件标识：kind 不符直接拒绝导入
pub const CONFIG_KIND: &str = "zap-config";
/// 配置文件格式版本：文件版本高于当前时拒绝（防旧版误读新版文件）
pub const CONFIG_VERSION: u32 = 1;

/// 配置文件内的设置项。全部可选：None = 导入时保留当前值——
/// 新版 zap 往文件里加键，旧版照常读，缺的键不动现设置
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSettings {
    pub hotkey: Option<String>,
    pub vault_hotkey: Option<String>,
    pub clip_hotkey: Option<String>,
    pub superpanel_hotkey: Option<String>,
    pub stage_hotkey: Option<String>,
    pub clip_enabled: Option<bool>,
    pub clip_max: Option<i64>,
    pub clip_autopaste: Option<bool>,
    pub stage_max: Option<i64>,
    pub stagedrop_enabled: Option<bool>,
    pub stagedrop_open_panel: Option<bool>,
    pub stagedrop_position: Option<String>,
    pub pwd_clear_secs: Option<i64>,
    pub autostart: Option<bool>,
    pub everything_autostart: Option<bool>,
    pub appearance: Option<String>,
    pub panel_transparency: Option<bool>,
}

/// 配置文件里的插件开关：仅 id + enabled，标题/入口词由本机插件 meta 下发
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPluginSwitch {
    pub id: String,
    pub enabled: bool,
}

/// 配置文件整体（JSON，pretty 序列化）。账号库数据刻意不含——那有口令加密的
/// 「备份与恢复」通道，两类文件不混用；剪贴板历史与中转站条目属即时数据同样不含
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFile {
    pub kind: String,
    pub version: u32,
    pub exported_at: Option<String>,
    pub app_version: Option<String>,
    #[serde(default)]
    pub settings: ConfigSettings,
    #[serde(default)]
    pub plugins: Vec<ConfigPluginSwitch>,
    #[serde(default)]
    pub shortcuts: Vec<super::shortcuts::Shortcut>,
}

/// 解析并校验配置文件文本：JSON 合法 + kind/version 匹配
pub fn validate_config_text(text: &str) -> Result<ConfigFile, String> {
    let file: ConfigFile =
        serde_json::from_str(text).map_err(|e| format!("不是有效的 zap 配置文件：{e}"))?;
    if file.kind != CONFIG_KIND {
        return Err("不是 zap 配置文件（文件标识不符）".into());
    }
    if file.version > CONFIG_VERSION {
        return Err(format!(
            "配置文件版本较新（v{version}），请先把 zap 升级到对应版本再导入",
            version = file.version
        ));
    }
    Ok(file)
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

    fn sample_file() -> ConfigFile {
        ConfigFile {
            kind: CONFIG_KIND.into(),
            version: CONFIG_VERSION,
            exported_at: Some("2026-10-27 12:00:00".into()),
            app_version: Some("0.11.0".into()),
            settings: ConfigSettings {
                hotkey: Some("ctrl+alt+space".into()),
                clip_max: Some(500),
                pwd_clear_secs: Some(45),
                appearance: Some("dark".into()),
                ..Default::default()
            },
            plugins: vec![ConfigPluginSwitch {
                id: "texttools".into(),
                enabled: false,
            }],
            shortcuts: vec![crate::providers::shortcuts::Shortcut {
                keyword: "gh".into(),
                name: "GitHub".into(),
                url: "https://github.com/search?q={query}".into(),
            }],
        }
    }

    #[test]
    fn 配置文件序列化往返() {
        let text = serde_json::to_string_pretty(&sample_file()).unwrap();
        let back = validate_config_text(&text).unwrap();
        assert_eq!(back.kind, CONFIG_KIND);
        assert_eq!(back.settings.hotkey.as_deref(), Some("ctrl+alt+space"));
        assert_eq!(back.settings.clip_max, Some(500));
        assert_eq!(back.settings.pwd_clear_secs, Some(45));
        assert_eq!(back.settings.appearance.as_deref(), Some("dark"));
        assert_eq!(back.plugins.len(), 1);
        assert!(!back.plugins[0].enabled);
        assert_eq!(back.shortcuts[0].keyword, "gh");
        // camelCase 线协议（前端消费的字段名）
        assert!(text.contains("\"clipMax\""));
        assert!(text.contains("\"pwdClearSecs\""));
        assert!(text.contains("\"stagedropOpenPanel\""));
    }

    #[test]
    fn 配置文件校验拒绝坏输入() {
        assert!(validate_config_text("not json").is_err());
        // kind 不符
        let mut f = sample_file();
        f.kind = "other".into();
        let text = serde_json::to_string(&f).unwrap();
        assert!(validate_config_text(&text).is_err());
        // 未来版本
        let mut f = sample_file();
        f.version = CONFIG_VERSION + 1;
        let text = serde_json::to_string(&f).unwrap();
        assert!(validate_config_text(&text).is_err());
    }

    #[test]
    fn 配置文件缺段容错() {
        // 只带 kind/version 的极简文件（旧格式/手工精简）合法，settings 为默认空
        let text = format!(r#"{{"kind":"{CONFIG_KIND}","version":{CONFIG_VERSION}}}"#);
        let back = validate_config_text(&text).unwrap();
        assert_eq!(back.settings, ConfigSettings::default());
        assert!(back.plugins.is_empty());
        assert!(back.shortcuts.is_empty());
    }

    #[test]
    fn 配置文件旧版缺键容错() {
        // 旧版 zap 导出的 v1 文件不含后加的键（如 stagedropOpenPanel）：
        // Option 字段缺失反序列化为 None = 导入时保留当前值（前向兼容契约）
        let text = format!(
            r#"{{"kind":"{CONFIG_KIND}","version":{CONFIG_VERSION},"settings":{{"hotkey":"ctrl+alt+space"}}}}"#
        );
        let back = validate_config_text(&text).unwrap();
        assert_eq!(back.settings.hotkey.as_deref(), Some("ctrl+alt+space"));
        assert_eq!(back.settings.stagedrop_open_panel, None);
        assert_eq!(back.settings.stagedrop_enabled, None);
        assert_eq!(back.settings.stagedrop_position, None);
    }
}
