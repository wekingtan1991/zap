//! 系统设置直达：索引 Windows 设置页（ms-settings: URI），中文/英文/缩写均可命中

use super::SearchItem;

struct SettingEntry {
    /// 展示名（中文）
    title: &'static str,
    /// ms-settings: URI
    uri: &'static str,
    /// 附加关键词（英文/缩写），空串表示无
    keywords: &'static str,
}

macro_rules! setting {
    ($title:expr, $uri:expr, $kw:expr) => {
        SettingEntry {
            title: $title,
            uri: $uri,
            keywords: $kw,
        }
    };
}

const SETTINGS: &[SettingEntry] = &[
    setting!("Windows 设置（主页）", "ms-settings:", "settings"),
    setting!("显示", "ms-settings:display", "display"),
    setting!("声音", "ms-settings:sound", "sound volume 音量"),
    setting!("通知", "ms-settings:notifications", "notification"),
    setting!("专注助手", "ms-settings:quiethours", "focus assist"),
    setting!("蓝牙", "ms-settings:bluetooth", "bluetooth"),
    setting!("WLAN 网络", "ms-settings:network-wifi", "wifi wlan wireless 无线"),
    setting!("以太网", "ms-settings:network-ethernet", "ethernet"),
    setting!("代理", "ms-settings:network-proxy", "proxy"),
    setting!("VPN", "ms-settings:network-vpn", "vpn"),
    setting!("飞行模式", "ms-settings:network-airplanemode", "airplane"),
    setting!("移动热点", "ms-settings:network-mobilehotspot", "hotspot 热点"),
    setting!("电源和睡眠", "ms-settings:powersleep", "power sleep"),
    setting!("电池", "ms-settings:batterysaver", "battery"),
    setting!("应用和功能", "ms-settings:appsfeatures", "apps uninstall 卸载"),
    setting!("默认应用", "ms-settings:defaultapps", "default apps"),
    setting!("启动应用", "ms-settings:startupapps", "startup 自启"),
    setting!("字体", "ms-settings:fonts", "fonts"),
    setting!("鼠标", "ms-settings:mouse", "mouse"),
    setting!("键盘", "ms-settings:keyboard", "keyboard"),
    setting!("打印机和扫描仪", "ms-settings:printers", "printer scanner 打印"),
    setting!("Windows 更新", "ms-settings:windowsupdate", "update 更新"),
    setting!("Windows 安全中心", "ms-settings:windowsdefender", "defender security 杀毒"),
    setting!("备份", "ms-settings:backup", "backup"),
    setting!("恢复", "ms-settings:recovery", "recovery"),
    setting!("关于（系统信息）", "ms-settings:about", "about system 系统信息"),
    setting!("存储", "ms-settings:storagesense", "storage 磁盘"),
    setting!("磁盘清理", "ms-settings:storagepolicies", "cleanup"),
    setting!("剪贴板", "ms-settings:clipboard", "clipboard"),
    setting!("多任务", "ms-settings:multitasking", "multitasking"),
    setting!("投影", "ms-settings:project", "project 投屏"),
    setting!("远程桌面", "ms-settings:remotedesktop", "remote desktop"),
    setting!("开发者选项", "ms-settings:developers", "developer"),
    setting!("个性化", "ms-settings:personalization", "personalization"),
    setting!("桌面背景", "ms-settings:personalization-background", "wallpaper background 壁纸"),
    setting!("颜色（深色模式）", "ms-settings:colors", "colors dark mode 深色"),
    setting!("锁屏界面", "ms-settings:lockscreen", "lockscreen"),
    setting!("主题", "ms-settings:themes", "themes"),
    setting!("任务栏", "ms-settings:taskbar", "taskbar"),
    setting!("日期和时间", "ms-settings:dateandtime", "date time datetime 时钟"),
    setting!("语言", "ms-settings:keyboard-options", "language input 输入法"),
    setting!("区域", "ms-settings:region", "region"),
    setting!("麦克风隐私", "ms-settings:privacy-microphone", "microphone"),
    setting!("摄像头隐私", "ms-settings:privacy-webcam", "webcam camera"),
    setting!("位置隐私", "ms-settings:privacy-location", "location 定位"),
    setting!("辅助功能", "ms-settings:easeofaccess", "accessibility"),
];

pub fn search(query: &str, limit: usize) -> Vec<SearchItem> {
    let mut scored: Vec<(f32, usize)> = SETTINGS
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            // 所有条目追加"设置/settings"关键词：搜"设置"可浏览全部设置页
            let hay = if e.keywords.is_empty() {
                format!("{} 设置 settings", e.title)
            } else {
                format!("{} {} 设置 settings", e.title, e.keywords)
            };
            super::score_match(&hay, query).map(|s| (s - 45.0, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);

    scored
        .into_iter()
        .map(|(score, i)| {
            let e = &SETTINGS[i];
            SearchItem {
                id: format!("settings:{i}"),
                title: e.title.to_string(),
                subtitle: "系统设置".into(),
                kind: "settings".into(),
                target: e.uri.to_string(),
                icon: None,
                score,
            }
        })
        .collect()
}
