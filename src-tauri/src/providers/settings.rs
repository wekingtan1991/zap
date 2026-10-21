//! 系统设置直达：索引 Windows 设置页（ms-settings: URI），中文/英文/缩写均可命中

use std::sync::LazyLock;

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
    setting!(
        "WLAN 网络",
        "ms-settings:network-wifi",
        "wifi wlan wireless 无线"
    ),
    setting!("以太网", "ms-settings:network-ethernet", "ethernet"),
    setting!("代理", "ms-settings:network-proxy", "proxy"),
    setting!("VPN", "ms-settings:network-vpn", "vpn"),
    setting!("飞行模式", "ms-settings:network-airplanemode", "airplane"),
    setting!(
        "移动热点",
        "ms-settings:network-mobilehotspot",
        "hotspot 热点"
    ),
    setting!("电源和睡眠", "ms-settings:powersleep", "power sleep"),
    setting!("电池", "ms-settings:batterysaver", "battery"),
    setting!(
        "应用和功能",
        "ms-settings:appsfeatures",
        "apps uninstall 卸载"
    ),
    setting!("默认应用", "ms-settings:defaultapps", "default apps"),
    setting!("启动应用", "ms-settings:startupapps", "startup 自启"),
    setting!("字体", "ms-settings:fonts", "fonts"),
    setting!("鼠标", "ms-settings:mouse", "mouse"),
    setting!("键盘", "ms-settings:keyboard", "keyboard"),
    setting!(
        "打印机和扫描仪",
        "ms-settings:printers",
        "printer scanner 打印"
    ),
    setting!("Windows 更新", "ms-settings:windowsupdate", "update 更新"),
    setting!(
        "Windows 安全中心",
        "ms-settings:windowsdefender",
        "defender security 杀毒"
    ),
    setting!("备份", "ms-settings:backup", "backup"),
    setting!("恢复", "ms-settings:recovery", "recovery"),
    setting!(
        "关于（系统信息）",
        "ms-settings:about",
        "about system 系统信息"
    ),
    setting!("存储", "ms-settings:storagesense", "storage 磁盘"),
    setting!("磁盘清理", "ms-settings:storagepolicies", "cleanup"),
    setting!("剪贴板", "ms-settings:clipboard", "clipboard"),
    setting!("多任务", "ms-settings:multitasking", "multitasking"),
    setting!("投影", "ms-settings:project", "project 投屏"),
    setting!("远程桌面", "ms-settings:remotedesktop", "remote desktop"),
    setting!("开发者选项", "ms-settings:developers", "developer"),
    setting!("个性化", "ms-settings:personalization", "personalization"),
    setting!(
        "桌面背景",
        "ms-settings:personalization-background",
        "wallpaper background 壁纸"
    ),
    setting!(
        "颜色（深色模式）",
        "ms-settings:colors",
        "colors dark mode 深色"
    ),
    setting!("锁屏界面", "ms-settings:lockscreen", "lockscreen"),
    setting!("主题", "ms-settings:themes", "themes"),
    setting!("任务栏", "ms-settings:taskbar", "taskbar"),
    setting!(
        "日期和时间",
        "ms-settings:dateandtime",
        "date time datetime 时钟"
    ),
    setting!(
        "语言",
        "ms-settings:keyboard-options",
        "language input 输入法"
    ),
    setting!("区域", "ms-settings:region", "region"),
    setting!("麦克风隐私", "ms-settings:privacy-microphone", "microphone"),
    setting!("摄像头隐私", "ms-settings:privacy-webcam", "webcam camera"),
    setting!("位置隐私", "ms-settings:privacy-location", "location 定位"),
    setting!("辅助功能", "ms-settings:easeofaccess", "accessibility"),
    // ---- 经典系统工具（非 ms-settings: 协议，走 ShellExecute / cmd start 打开）----
    // 控制面板、设备管理器这类系统入口在 Windows 10/11 已不在开始菜单，
    // ms-settings: 也没有对应页，因此单独收录，target 用系统自带可执行名/管理单元。
    setting!("控制面板", "control.exe", "control panel"),
    setting!("设备管理器", "hdwwiz.cpl", "device manager设备"),
    setting!("计算机管理", "compmgmt.msc", "computer management计算机"),
    setting!("服务", "services.msc", "services服务"),
    setting!("事件查看器", "eventvwr.msc", "event viewer log日志"),
    setting!("任务计划程序", "taskschd.msc", "task scheduler计划"),
    setting!("本地用户和组", "lusrmgr.msc", "local users groups用户组"),
    setting!("磁盘管理", "diskmgmt.msc", "disk management磁盘"),
    setting!("系统属性", "sysdm.cpl", "system properties属性"),
    setting!("用户账户", "userpasswords2.cpl", "user account用户"),
    setting!("程序和功能", "appwiz.cpl", "programs features卸载程序"),
    setting!("系统还原", "dfrgui.cpl", "system restore还原"),
    setting!("Internet 选项", "inetcpl.cpl", "internet options"),
    setting!("电源选项", "powercfg.cpl", "power options电源"),
    setting!("任务管理器", "taskmgr", "task manager任务"),
    setting!("注册表编辑器", "regedit", "registry注册表"),
    setting!("命令提示符", "cmd", "command prompt命令"),
    setting!("PowerShell", "powershell", "powershell"),
    setting!("系统信息", "msinfo32", "msinfo32 system info"),
    setting!("性能监视器", "perfmon", "performance monitor性能"),
    setting!("资源监视器", "resmon", "resource monitor资源"),
    setting!("DirectX 诊断", "dxdiag", "directx"),
    setting!("远程桌面连接", "mstsc", "mstsc remote desktop"),
];

/// 静态表的匹配形态（含追加的"设置/settings"关键词），首次搜索时构建一次
static FORMS: LazyLock<Vec<super::MatchForms>> = LazyLock::new(|| {
    SETTINGS
        .iter()
        .map(|e| {
            let hay = if e.keywords.is_empty() {
                format!("{} 设置 settings", e.title)
            } else {
                format!("{} {} 设置 settings", e.title, e.keywords)
            };
            super::MatchForms::new(&hay)
        })
        .collect()
});

pub fn search(query: &str, limit: usize) -> Vec<SearchItem> {
    let q = query.trim().to_lowercase();
    let mut scored: Vec<(f32, usize)> = FORMS
        .iter()
        .enumerate()
        .filter_map(|(i, forms)| forms.score(&q).map(|s| (s - 45.0, i)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 控制面板可被搜到() {
        let res = search("控制面板", 10);
        assert!(
            res.iter().any(|i| i.title == "控制面板"),
            "搜\"控制面板\"应命中：{res:?}"
        );
    }

    #[test]
    fn 经典系统工具支持中英文与拼音() {
        for q in ["控制面板", "control panel", "kongzhimianban", "kzb"] {
            assert!(
                search(q, 30).iter().any(|i| i.title == "控制面板"),
                "「{q}」应命中控制面板"
            );
        }
        assert!(
            search("设备管理器", 10)
                .iter()
                .any(|i| i.title == "设备管理器")
        );
        assert!(
            search("computer", 20)
                .iter()
                .any(|i| i.title == "计算机管理")
        );
    }

    #[test]
    fn ms_设置页仍走协议且分类正确() {
        let res = search("显示", 10);
        let item = res.iter().find(|i| i.title == "显示").expect("应命中显示");
        assert_eq!(item.target, "ms-settings:display");
        assert_eq!(item.kind, "settings");
    }
}
