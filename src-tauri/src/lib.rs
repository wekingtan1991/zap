mod hotkey_capture;
mod providers;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use providers::{SearchEngine, SearchItem};
use tauri::{
    AppHandle, Emitter, Manager, WindowEvent,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{
    Builder as GlobalShortcutBuilder, GlobalShortcutExt, Shortcut, ShortcutState,
};
use tauri_plugin_opener::OpenerExt;

const WINDOW_LABEL: &str = "main";
const PANEL_WIDTH: f64 = 720.0;
const PANEL_HEIGHT: f64 = 480.0;

/// 直达热键的动作：账号库 = 呼出面板并进入 pw 保护域；快速粘贴 = 不经面板，
/// 把最近一条剪贴板内容粘贴到当前前台窗口；超级面板 = 抓取选中文本出动作菜单；
/// 中转站 = 呼出面板并直接落中转站视图
#[derive(Clone, Copy)]
enum DirectKind {
    Vault,
    QuickPaste,
    SuperPanel,
    Stage,
}

struct AppState {
    engine: std::sync::Arc<SearchEngine>,
    /// 当前生效的呼出热键（设置保存时热切换）
    hotkey: Mutex<String>,
    /// 直达热键表：规范化热键串 → 动作（设置保存时热更新）
    direct_hotkeys: Mutex<HashMap<String, DirectKind>>,
    /// 托盘「记录剪贴板历史」勾选项，设置页改动时同步勾选态
    clip_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    /// 面板最近一次显示的时间，用于忽略显示瞬间的焦点抖动
    shown_at: Mutex<Option<Instant>>,
    /// 安装流程期间保持面板可见（UAC 等弹窗会抢焦点）
    hold_panel: std::sync::atomic::AtomicBool,
    /// 呼出面板时的前台窗口：剪贴板条目「选中即粘贴」要交还焦点的目标
    prev_foreground: Mutex<Option<isize>>,
}

/// spawn 控制台子进程（cmd/powershell）：GUI 进程下不闪黑框
fn spawn_hidden(command: &mut std::process::Command) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn()
}

#[tauri::command]
async fn search(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<SearchItem>, String> {
    // 搜索在阻塞线程池执行：SQLite、窗口枚举、Everything HTTP 都不占主线程
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.search(&query))
        .await
        .map_err(|e| e.to_string())
}

/// 隐藏面板并通知前端（zap://hidden）。
/// 前端在**收起时**就清空搜索词，这样下次呼出天然是空状态——
/// 若等到 show 之后才清，用户会先看到一帧旧内容再被擦掉（表现为"闪一下"）。
fn hide_panel_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(WINDOW_LABEL) {
        let _ = win.hide();
    }
    // 任何隐藏路径都释放 hold：中转站视图借用 hold 对抗失焦即隐，
    // 面板已隐还挂着 hold 会变成幽灵态（下次呼出失焦不隐）
    app.state::<AppState>()
        .hold_panel
        .store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = app.emit("zap://hidden", ());
}

#[tauri::command]
fn hide_panel(app: AppHandle) {
    hide_panel_window(&app);
}

#[tauri::command]
fn set_hold_panel(state: tauri::State<AppState>, hold: bool) {
    use std::sync::atomic::Ordering;
    state.hold_panel.store(hold, Ordering::Relaxed);
}

// 除面板开关类轻量命令外，涉及 DB/DPAPI/剪贴板/HTTP 的命令一律 async +
// spawn_blocking：Tauri 同步命令跑在主线程，SQLite 与剪贴板重试窗口会卡住
// 窗口消息泵（everything_status 的端口探测最坏可达数百毫秒）

#[tauri::command]
async fn everything_status() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(providers::everything::status)
        .await
        .map_err(|e| e.to_string())
}

/// 自动检测并在需要时启动 Everything。按设置开关放行；已安装才启动，
/// 未安装不自动下载（留给面板里的手动入口）；失败 5 分钟内不重复尝试。
#[tauri::command]
async fn everything_ensure(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let auto = state
        .engine
        .get_setting(providers::appconfig::KEY_EVERYTHING_AUTOSTART)
        .map(|v| v != "0")
        .unwrap_or(providers::appconfig::DEFAULT_EVERYTHING_AUTOSTART);
    if !auto {
        return Ok("disabled".into());
    }
    tauri::async_runtime::spawn_blocking(providers::everything::ensure)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn vault_search(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<SearchItem>, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.vault_search(&query))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn vault_add(
    state: tauri::State<'_, AppState>,
    title: String,
    username: String,
    password: String,
    url: String,
    notes: String,
    totp: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.vault_add(&title, &username, &password, &url, &notes, &totp)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn vault_action(
    state: tauri::State<'_, AppState>,
    id: i64,
    action: String,
) -> Result<String, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let secs: i64 = engine
            .get_setting(providers::appconfig::KEY_PWD_CLEAR_SECS)
            .and_then(|v| v.parse().ok())
            .unwrap_or(providers::appconfig::DEFAULT_PWD_CLEAR_SECS);
        engine.vault_action(id, &action, secs)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 取单条账号完整记录（编辑预填；含解密后的密码明文与 TOTP 密钥）
#[tauri::command]
async fn vault_get(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<providers::vault::VaultRecord, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.vault_get(id))
        .await
        .map_err(|e| e.to_string())?
}

// Tauri 命令按字段平铺（前端 invoke 传同名参数），显式放行参数个数提示
#[allow(clippy::too_many_arguments)]
#[tauri::command]
async fn vault_update(
    state: tauri::State<'_, AppState>,
    id: i64,
    title: String,
    username: String,
    password: String,
    url: String,
    notes: String,
    pinned: i64,
    totp: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.vault_update(
            id, &title, &username, &password, &url, &notes, pinned, &totp,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- 应用设置 ----------

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsDto {
    hotkey: String,
    /// 账号库直达热键（空串 = 未绑定）
    vault_hotkey: String,
    /// 快速粘贴直达热键（空串 = 未绑定）
    clip_hotkey: String,
    /// 超级面板直达热键（空串 = 未绑定）
    superpanel_hotkey: String,
    /// 中转站直达热键（空串 = 未绑定）
    stage_hotkey: String,
    clip_enabled: bool,
    clip_max: i64,
    /// 剪贴板条目选中后自动粘贴到原窗口
    clip_autopaste: bool,
    /// 中转站未固定条目容量上限
    stage_max: i64,
    /// 全局拖拽落区条开关
    stagedrop_enabled: bool,
    pwd_clear_secs: i64,
    autostart: bool,
    everything_autostart: bool,
    /// 外观模式（dark/light/system）
    appearance: String,
    /// 透明面板（亚克力窗口特效）
    panel_transparency: bool,
    /// 可开关插件清单（仅工具插件；核心搜索源不列）
    plugins: Vec<providers::PluginInfo>,
}

/// 组装设置快照：注册表读（autostart）与 SQLite 读都属磁盘 IO，抽出来给
/// 异步命令在阻塞线程池执行
fn build_app_settings(
    app: &AppHandle,
    engine: &std::sync::Arc<SearchEngine>,
    hotkey: &str,
) -> AppSettingsDto {
    AppSettingsDto {
        hotkey: hotkey.to_string(),
        vault_hotkey: engine
            .get_setting(providers::appconfig::KEY_VAULT_HOTKEY)
            .unwrap_or_default(),
        clip_hotkey: engine
            .get_setting(providers::appconfig::KEY_CLIP_HOTKEY)
            .unwrap_or_default(),
        superpanel_hotkey: engine
            .get_setting(providers::appconfig::KEY_SUPERPANEL_HOTKEY)
            .unwrap_or_default(),
        stage_hotkey: engine
            .get_setting(providers::appconfig::KEY_STAGE_HOTKEY)
            .unwrap_or_default(),
        clip_enabled: providers::clipboard::CLIP_ENABLED.load(Ordering::Relaxed),
        clip_max: providers::clipboard::RETENTION_LIMIT.load(Ordering::Relaxed),
        clip_autopaste: providers::clipboard::PASTE_TO_PREV.load(Ordering::Relaxed),
        stage_max: providers::stage::CAP.load(Ordering::Relaxed),
        stagedrop_enabled: providers::stagedrop::STAGEDROP_ENABLED.load(Ordering::Relaxed),
        pwd_clear_secs: engine
            .get_setting(providers::appconfig::KEY_PWD_CLEAR_SECS)
            .and_then(|v| v.parse().ok())
            .unwrap_or(providers::appconfig::DEFAULT_PWD_CLEAR_SECS),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        everything_autostart: engine
            .get_setting(providers::appconfig::KEY_EVERYTHING_AUTOSTART)
            .map(|v| v != "0")
            .unwrap_or(providers::appconfig::DEFAULT_EVERYTHING_AUTOSTART),
        appearance: engine
            .get_setting(providers::appconfig::KEY_APPEARANCE)
            .unwrap_or_else(|| "dark".into()),
        panel_transparency: engine
            .get_setting(providers::appconfig::KEY_PANEL_TRANSPARENCY)
            .map(|v| v != "0")
            .unwrap_or(providers::appconfig::DEFAULT_PANEL_TRANSPARENCY),
        plugins: engine.plugin_states(),
    }
}

/// 设置读取：注册表 + SQLite 读不占主线程（async 命令带 State 须返回
/// Result，Tauri 约定；前端调用方本就带 catch 兜底）
#[tauri::command]
async fn get_app_settings(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<AppSettingsDto, String> {
    let engine = state.engine.clone();
    let hotkey = state.hotkey.lock().expect("hotkey poisoned").clone();
    tauri::async_runtime::spawn_blocking(move || Ok(build_app_settings(&app, &engine, &hotkey)))
        .await
        .map_err(|e| e.to_string())?
}

/// 插件开关（save_settings 入参）
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginSwitchDto {
    id: String,
    enabled: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveOutcome {
    /// 热键保存失败的原因；None = 生效
    hotkey_error: Option<String>,
    /// 直达热键保存失败的原因；None = 生效（保存其余设置不受影响）
    direct_hotkey_error: Option<String>,
}

// Tauri 命令按字段平铺（前端 invoke 传同名参数），显式放行参数个数提示。
// 刻意保持同步命令（主线程执行）：global-shortcut 的注册/注销与创建它的主
// 线程消息队列绑定（RegisterHotKey 线程亲和），挪进 spawn_blocking 会让
// 热键静默失效。其中的 SQLite 写是 WAL 毫秒级操作，不构成 lib.rs 顶部军规
// 针对的"卡消息泵"量级
#[allow(clippy::too_many_arguments)]
#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: tauri::State<AppState>,
    hotkey: String,
    // Option 容错：半新半旧的会话（前端未带上新键）保存时不再整单被拒
    vault_hotkey: Option<String>,
    clip_hotkey: Option<String>,
    superpanel_hotkey: Option<String>,
    stage_hotkey: Option<String>,
    clip_enabled: bool,
    clip_max: i64,
    clip_autopaste: Option<bool>,
    // Option 容错：半新半旧的会话（前端未带上新键）保存时不再整单被拒
    stage_max: Option<i64>,
    stagedrop_enabled: Option<bool>,
    pwd_clear_secs: i64,
    everything_autostart: bool,
    appearance: Option<String>,
    plugins: Vec<PluginSwitchDto>,
) -> Result<SaveOutcome, String> {
    let hotkey = hotkey.trim().to_lowercase();
    let mut hotkey_error = None;
    let current = state.hotkey.lock().expect("hotkey poisoned").clone();
    if current != hotkey {
        let gs = app.global_shortcut();
        // 先解绑旧键；新键注册失败则回滚旧键并报错，其他设置照常保存
        if let Ok(old) = current.parse::<Shortcut>() {
            let _ = gs.unregister(old);
        }
        match hotkey.parse::<Shortcut>() {
            Ok(sc) => match gs.register(sc) {
                Ok(_) => {
                    eprintln!("zap: 呼出热键已切换 {current} → {hotkey}");
                    state
                        .engine
                        .set_setting(providers::appconfig::KEY_HOTKEY, &hotkey)?;
                    *state.hotkey.lock().expect("hotkey poisoned") = hotkey.clone();
                }
                Err(e) => {
                    eprintln!("zap: 热键 {hotkey} 注册失败: {e}");
                    hotkey_error = Some(format!("注册失败，可能已被其他程序占用：{e}"));
                    if let Ok(old) = current.parse::<Shortcut>() {
                        let _ = gs.register(old);
                    }
                }
            },
            Err(e) => {
                eprintln!("zap: 热键 {hotkey} 无法解析: {e}");
                hotkey_error = Some(format!("无法识别的按键组合：{e}"));
                if let Ok(old) = current.parse::<Shortcut>() {
                    let _ = gs.register(old);
                }
            }
        }
    }

    // 直达热键槽位：None = 旧前端未带键跳过；空串 = 解绑；其余 = 注册
    // （失败逐条记入 direct_errors 不阻塞其余保存，与主热键同口径）
    let mut direct_errors: Vec<String> = Vec::new();
    // 已被占用的键：主呼出热键 + 先处理的直达槽位，后槽位同键直接拒绝
    // （否则槽位间冲突会上游报「被其他程序占用」，实为自家槽位，误导）
    let mut taken = vec![hotkey.clone()];
    {
        let gs = app.global_shortcut();
        for (name, raw, cfg_key, kind) in [
            (
                "账号库直达",
                vault_hotkey,
                providers::appconfig::KEY_VAULT_HOTKEY,
                DirectKind::Vault,
            ),
            (
                "快速粘贴",
                clip_hotkey,
                providers::appconfig::KEY_CLIP_HOTKEY,
                DirectKind::QuickPaste,
            ),
            (
                "超级面板",
                superpanel_hotkey,
                providers::appconfig::KEY_SUPERPANEL_HOTKEY,
                DirectKind::SuperPanel,
            ),
            (
                "中转站直达",
                stage_hotkey,
                providers::appconfig::KEY_STAGE_HOTKEY,
                DirectKind::Stage,
            ),
        ] {
            let Some(raw) = raw else { continue };
            let norm = raw.trim().to_lowercase();
            let mut map = state.direct_hotkeys.lock().expect("direct poisoned");
            // 旧绑定先解绑并移出热键表（无论换成什么值），死键不能留到重启
            let old = state
                .engine
                .get_setting(cfg_key)
                .unwrap_or_default()
                .trim()
                .to_lowercase();
            if let Ok(old_sc) = old.parse::<Shortcut>() {
                let _ = gs.unregister(old_sc);
            }
            if norm.is_empty() {
                state.engine.set_setting(cfg_key, "")?;
                map.remove(&old);
                continue;
            }
            // 同键冲突：拒绝并恢复原绑定（旧绑定已解绑，不恢复就丢了）
            if taken.contains(&norm) {
                direct_errors.push(format!(
                    "「{name}」热键不能与呼出热键相同，各直达热键之间也不能相同"
                ));
                if let Ok(old_sc) = old.parse::<Shortcut>() {
                    let _ = gs.register(old_sc);
                }
                continue;
            }
            match norm.parse::<Shortcut>() {
                Ok(sc) => match gs.register(sc) {
                    Ok(_) => {
                        state.engine.set_setting(cfg_key, &norm)?;
                        map.remove(&old);
                        map.insert(norm.clone(), kind);
                        taken.push(norm);
                    }
                    Err(e) => {
                        direct_errors
                            .push(format!("「{name}」热键注册失败，可能已被其他程序占用：{e}"));
                        // 恢复旧绑定
                        if let Ok(old_sc) = old.parse::<Shortcut>() {
                            let _ = gs.register(old_sc);
                        }
                    }
                },
                Err(e) => {
                    direct_errors.push(format!("「{name}」：无法识别的按键组合：{e}"));
                    if let Ok(old_sc) = old.parse::<Shortcut>() {
                        let _ = gs.register(old_sc);
                    }
                }
            }
        }
    }

    providers::clipboard::CLIP_ENABLED.store(clip_enabled, Ordering::Relaxed);
    providers::clipboard::RETENTION_LIMIT.store(clip_max, Ordering::Relaxed);
    state.engine.set_setting(
        providers::appconfig::KEY_CLIP_ENABLED,
        if clip_enabled { "1" } else { "0" },
    )?;
    state
        .engine
        .set_setting(providers::appconfig::KEY_CLIP_MAX, &clip_max.to_string())?;
    if let Some(on) = clip_autopaste {
        providers::clipboard::PASTE_TO_PREV.store(on, Ordering::Relaxed);
        state.engine.set_setting(
            providers::appconfig::KEY_CLIP_AUTOPASTE,
            if on { "1" } else { "0" },
        )?;
    }
    if let Some(cap) = stage_max {
        let cap = cap.clamp(10, 500);
        providers::stage::CAP.store(cap, Ordering::Relaxed);
        // 调小立即收敛已有超容条目：等下次放入才收敛不符合「改了就生效」直觉
        // （调大无需动作，多出的空间天然可用）
        state.engine.stage_enforce_cap();
        state
            .engine
            .set_setting(providers::appconfig::KEY_STAGE_CAP, &cap.to_string())?;
    }
    if let Some(on) = stagedrop_enabled {
        providers::stagedrop::STAGEDROP_ENABLED.store(on, Ordering::Relaxed);
        state.engine.set_setting(
            providers::appconfig::KEY_STAGEDROP_ENABLED,
            if on { "1" } else { "0" },
        )?;
    }
    if let Some(mode) = appearance {
        // 只收合法值（防前端脏数据把主题写坏）
        if matches!(mode.as_str(), "dark" | "light" | "system") {
            state
                .engine
                .set_setting(providers::appconfig::KEY_APPEARANCE, &mode)?;
        }
    }
    state.engine.set_setting(
        providers::appconfig::KEY_PWD_CLEAR_SECS,
        &pwd_clear_secs.to_string(),
    )?;
    state.engine.set_setting(
        providers::appconfig::KEY_EVERYTHING_AUTOSTART,
        if everything_autostart { "1" } else { "0" },
    )?;
    // 插件开关：逐个写存档并同步原子量（未知 id 直接报错，防前端脏数据静默丢失）
    for p in &plugins {
        state.engine.set_plugin_enabled(&p.id, p.enabled)?;
    }
    if let Some(item) = state.clip_item.lock().expect("clip_item poisoned").as_ref() {
        let _ = item.set_checked(clip_enabled);
    }
    let direct_hotkey_error = if direct_errors.is_empty() {
        None
    } else {
        Some(direct_errors.join("；"))
    };
    Ok(SaveOutcome {
        hotkey_error,
        direct_hotkey_error,
    })
}

#[tauri::command]
fn autostart_set(app: AppHandle, enable: bool) -> Result<(), String> {
    let al = app.autolaunch();
    if enable {
        al.enable().map_err(|e| e.to_string())
    } else {
        al.disable().map_err(|e| e.to_string())
    }
}

// ---------- 配置文件导出/导入 ----------

/// 设置快照 → 配置文件 settings 段（全量 Some：导出即当前生效值）
fn config_settings_from(dto: &AppSettingsDto) -> providers::appconfig::ConfigSettings {
    providers::appconfig::ConfigSettings {
        hotkey: Some(dto.hotkey.clone()),
        vault_hotkey: Some(dto.vault_hotkey.clone()),
        clip_hotkey: Some(dto.clip_hotkey.clone()),
        superpanel_hotkey: Some(dto.superpanel_hotkey.clone()),
        stage_hotkey: Some(dto.stage_hotkey.clone()),
        clip_enabled: Some(dto.clip_enabled),
        clip_max: Some(dto.clip_max),
        clip_autopaste: Some(dto.clip_autopaste),
        stage_max: Some(dto.stage_max),
        stagedrop_enabled: Some(dto.stagedrop_enabled),
        pwd_clear_secs: Some(dto.pwd_clear_secs),
        autostart: Some(dto.autostart),
        everything_autostart: Some(dto.everything_autostart),
        appearance: Some(dto.appearance.clone()),
        panel_transparency: Some(dto.panel_transparency),
    }
}

/// 导出配置文件：当前设置 + 插件开关 + 搜索捷径打包为 JSON。账号库不在此列
/// （走口令加密的备份恢复）。设置组装含注册表/SQLite 读，落盘走阻塞线程池
#[tauri::command]
async fn config_export(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    let hotkey = state.hotkey.lock().expect("hotkey poisoned").clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dto = build_app_settings(&app, &engine, &hotkey);
        let file = providers::appconfig::ConfigFile {
            kind: providers::appconfig::CONFIG_KIND.into(),
            version: providers::appconfig::CONFIG_VERSION,
            exported_at: Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
            app_version: Some(app.package_info().version.to_string()),
            settings: config_settings_from(&dto),
            plugins: dto
                .plugins
                .iter()
                .map(|p| providers::appconfig::ConfigPluginSwitch {
                    id: p.id.clone(),
                    enabled: p.enabled,
                })
                .collect(),
            shortcuts: engine.shortcuts_list(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        std::fs::write(&path, text + "\n").map_err(|e| format!("写入失败：{e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 导入配置文件：读文件 + 校验 kind/version 后原样返回，前端回填设置表单，
/// 用户核对后点「保存」走既有管线生效（热键冲突回滚、插件开关校验都在那条路上）
#[tauri::command]
async fn config_import(path: String) -> Result<providers::appconfig::ConfigFile, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("读取失败：{e}"))?;
        providers::appconfig::validate_config_text(&text)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 透明面板（亚克力）：应用/清除窗口特效。系统不支持时返回 Err（调用方决定回退）。
/// 特效只作用于桌面模糊底，面板自身透明度由前端 CSS（zap-trans 类）负责
fn apply_window_effects(app: &AppHandle, on: bool) -> Result<(), String> {
    use tauri::utils::config::WindowEffectsConfig;
    use tauri::utils::{WindowEffect, WindowEffectState};
    let Some(win) = app.get_webview_window(WINDOW_LABEL) else {
        return Err("面板窗口不存在".into());
    };
    let result = if on {
        win.set_effects(Some(WindowEffectsConfig {
            effects: vec![WindowEffect::Acrylic],
            state: Some(WindowEffectState::Active),
            ..Default::default()
        }))
    } else {
        win.set_effects(None)
    };
    result.map_err(|e| e.to_string())
}

/// 透明面板开关：应用/清除亚克力并持久化偏好；失败（系统不支持）不落偏好，
/// 前端据此回退为不透明并提示
#[tauri::command]
async fn transparency_set(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    enable: bool,
) -> Result<(), String> {
    apply_window_effects(&app, enable)?;
    state.engine.set_setting(
        providers::appconfig::KEY_PANEL_TRANSPARENCY,
        if enable { "1" } else { "0" },
    )
}

/// 超级面板动作执行（前端菜单点击；执行后由前端隐藏窗口）
#[tauri::command]
async fn superpanel_act(app: AppHandle, action: String, text: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        providers::superpanel::run_action(&app, &action, &text)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 超级面板当前菜单内容（前端聚焦窗口时拉取；事件推送之外的兜底通道）
#[tauri::command]
fn superpanel_payload() -> Option<providers::superpanel::SuperActionsPayload> {
    providers::superpanel::current_payload()
}

/// 收起超级面板（Esc / 点击项后调用）
#[tauri::command]
fn superpanel_hide(app: AppHandle) {
    providers::superpanel::superpanel_hide(&app);
}

// ---------- 热键录制（低级键盘钩子，可捕获被输入法接管的组合如 Ctrl+Space） ----------

#[tauri::command]
fn start_hotkey_capture(app: AppHandle) {
    hotkey_capture::start(app);
}

#[tauri::command]
fn stop_hotkey_capture() {
    hotkey_capture::stop();
}

#[tauri::command]
async fn vault_parse(text: String) -> Result<Vec<providers::vault::ParsedAccount>, String> {
    tauri::async_runtime::spawn_blocking(move || providers::vault::parse_blob(&text))
        .await
        .map_err(|e| e.to_string())
}

// ---------- JSON 工具插件 ----------

#[tauri::command]
async fn json_analyze(text: String) -> Result<providers::jsontools::JsonReport, String> {
    tauri::async_runtime::spawn_blocking(move || providers::jsontools::analyze(&text))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn json_format(text: String, pretty: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || providers::jsontools::format_text(&text, pretty))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn json_to_yaml(text: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || providers::jsontools::to_yaml(&text))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn json_to_ts(text: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || providers::jsontools::to_ts(&text))
        .await
        .map_err(|e| e.to_string())?
}

// ---------- 文本工具域 ----------

/// 一次返回全部转换（时间戳/Base64/URL/哈希），前端按卡片渲染
#[tauri::command]
async fn tool_convert_all(text: String) -> Result<providers::texttools::ToolBundle, String> {
    tauri::async_runtime::spawn_blocking(move || providers::texttools::convert_all(&text))
        .await
        .map_err(|e| e.to_string())?
}

/// 生成一枚 UUID v4（按需点击生成，不随输入刷新）
#[tauri::command]
async fn tool_uuid() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(providers::texttools::uuid_v4)
        .await
        .map_err(|e| e.to_string())?
}

/// 识别条目回查原文：按行号读剪贴板历史（进入工作台时以识别时刻的内容为准）
#[tauri::command]
async fn clip_row(state: tauri::State<'_, AppState>, id: i64) -> Result<String, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine
            .clip_row(id)
            .ok_or_else(|| "该条剪贴板已被清理".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 合并粘贴结果：count = 实际合并条数（已被清理的跳过）；pasted = 是否带了
/// 焦点交还+注入（自动粘贴关闭时只回填，措辞应说「到剪贴板」而非「粘贴」）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MergePasteOutcome {
    count: usize,
    pasted: bool,
}

/// 多条合并粘贴：按前端给定的点选顺序取各条全文，换行拼接后走与单条
/// 「选中即粘贴」同一套回填+注入流程（take_prev_foreground + paste_text_async）。
/// 合并产物是新组合，刻意不进自身复制抑制窗——与单条粘贴同口径，会收录进
/// 历史顶部。个别条目已被清理则跳过并返回实际条数（前端据此提示），
/// 全部已清理才报错
#[tauri::command]
async fn clip_merge_paste(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<MergePasteOutcome, String> {
    if ids.is_empty() {
        return Err("没有选择要合并的条目".into());
    }
    let engine = state.engine.clone();
    let texts = tauri::async_runtime::spawn_blocking(move || engine.clip_texts(&ids))
        .await
        .map_err(|e| e.to_string())?;
    if texts.is_empty() {
        return Err("所选条目都已被清理".into());
    }
    let prev = take_prev_foreground(&app);
    if prev.is_some() {
        hide_panel_window(&app);
    }
    paste_text_async(texts.join("\n"), prev);
    Ok(MergePasteOutcome {
        count: texts.len(),
        pasted: prev.is_some(),
    })
}

/// 通用文本复制（JSON 美化/压缩结果回填等）：
/// 走自身复制抑制窗，派生结果不重复进入剪贴板历史；写入带退避重试
#[tauri::command]
async fn copy_text(text: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        providers::clipboard::begin_self_copy();
        providers::clipboard::retrying(|| {
            arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.clone()))
        })
        .map_err(|e| e.to_string())?;
        Ok("已复制".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 图片预览面板数据：全图 PNG（base64）+ 元信息。历史保留期内行号恒有效，
/// 已清理返回错误（面板内呈现，不收面板）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipImagePreview {
    width: u32,
    height: u32,
    size: i64,
    created_at: i64,
    pinned: bool,
    png_base64: String,
}

/// 图片条目预览：预览面板进入时点查一次（与回贴同一行号寻址）
#[tauri::command]
async fn clipimg_preview(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<ClipImagePreview, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (w, h, size, created_at, pinned, png) = engine
            .clip_image_meta(id)
            .ok_or_else(|| "该条剪贴板图片已被清理".to_string())?;
        Ok(ClipImagePreview {
            width: w,
            height: h,
            size,
            created_at,
            pinned,
            png_base64: base64::engine::general_purpose::STANDARD.encode(png),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 图片仅复制不粘贴：历史原图放回剪贴板。走自身复制抑制窗——图已在历史，
/// 监听不应重复收录（与 copy_text 同口径）；解码与写入在后台线程
#[tauri::command]
async fn clipimg_copy(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (w, h, png) = engine
            .clip_image(id)
            .ok_or_else(|| "该条剪贴板图片已被清理".to_string())?;
        let (_, _, rgba) = providers::clipimage::decode_png(&png)?;
        providers::clipboard::begin_self_copy();
        providers::clipboard::set_image_dib(w as usize, h as usize, &rgba)?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 图片另存为 PNG：路径由前端 plugin-dialog save() 选定（与备份导出同模式），
/// 后端只负责把历史里的原始 PNG 落盘
#[tauri::command]
async fn clipimg_export(
    state: tauri::State<'_, AppState>,
    id: i64,
    path: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (_, _, png) = engine
            .clip_image(id)
            .ok_or_else(|| "该条剪贴板图片已被清理".to_string())?;
        std::fs::write(&path, png).map_err(|e| format!("写入失败：{e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 搜索捷径清单（设置页「搜索捷径」分区）：纯缓存读，无库访问
#[tauri::command]
fn shortcuts_list(state: tauri::State<AppState>) -> Vec<providers::shortcuts::Shortcut> {
    state.engine.shortcuts_list()
}

/// 整单保存搜索捷径：校验 + 写库 + 热替换搜索缓存（写库走线程池）
#[tauri::command]
async fn shortcuts_save(
    state: tauri::State<'_, AppState>,
    list: Vec<providers::shortcuts::Shortcut>,
) -> Result<(), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.shortcuts_save(list))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn vault_import(
    state: tauri::State<'_, AppState>,
    entries: Vec<providers::vault::ParsedAccount>,
) -> Result<usize, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.vault_import(entries))
        .await
        .map_err(|e| e.to_string())?
}

// ---------- 账号库备份（加密导出/恢复） ----------

/// 加密导出全部账号到用户选定路径，返回条目数
#[tauri::command]
async fn vault_backup_export(
    state: tauri::State<'_, AppState>,
    path: String,
    passphrase: String,
) -> Result<usize, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.vault_backup_export(&path, &passphrase))
        .await
        .map_err(|e| e.to_string())?
}

/// 解密备份文件为预览（条目 + 疑似重复标记，恢复前交前端确认）
#[tauri::command]
async fn vault_backup_decrypt(
    state: tauri::State<'_, AppState>,
    path: String,
    passphrase: String,
) -> Result<providers::backup::BackupPreview, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.vault_backup_decrypt(&path, &passphrase))
        .await
        .map_err(|e| e.to_string())?
}

/// 备份恢复：skip_duplicates 时跳过疑似重复，返回 (写入数, 跳过数)
#[tauri::command]
async fn vault_backup_restore(
    state: tauri::State<'_, AppState>,
    entries: Vec<providers::backup::BackupEntry>,
    skip_duplicates: bool,
) -> Result<(usize, usize), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.vault_backup_restore(entries, skip_duplicates)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 条目次级操作：admin=管理员运行 / reveal=所在文件夹定位 / copy_path=复制路径或链接 /
/// clip_copy=剪贴板条目仅复制（不粘贴） / close_window=关闭窗口。
/// .lnk（含 recent 的指针）先解析为真实目标；商店应用（shell:）不支持路径类操作
#[tauri::command]
async fn secondary_action(
    state: tauri::State<'_, AppState>,
    action: String,
    target: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 非路径类操作先行：不参与 shell: 拦截与 lnk 解析
        match action.as_str() {
            // 历史条目按 rowid 回查全文回填（与选中即粘贴同口径：监听去重自动置顶），
            // 不交还原窗口、不注入 Ctrl+V
            "clip_copy" => {
                let id: i64 = target.parse().map_err(|_| "无效条目".to_string())?;
                let text = engine
                    .clip_row(id)
                    .ok_or_else(|| "该条剪贴板已被清理".to_string())?;
                providers::clipboard::retrying(|| {
                    arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.clone()))
                })
                .map(|_| ())
            }
            "close_window" => {
                let hwnd: isize = target.parse().map_err(|_| "无效窗口".to_string())?;
                providers::sysutil::close_window(hwnd)
            }
            _ => {
                if target.starts_with("shell:") {
                    return Err("商店应用不支持该操作".to_string());
                }
                let resolved =
                    providers::apps::resolve_lnk_target(&target).unwrap_or_else(|| target.clone());
                match action.as_str() {
                    "admin" => {
                        engine.record_app_launch(&target);
                        // 解析目标不存在时回退 lnk 本体：ShellExecute 由 shell 自行解析快捷方式，
                        // 能兜住自解析拿不到有效路径的情形
                        let exe = if std::path::Path::new(&resolved).exists() {
                            resolved.clone()
                        } else {
                            target.clone()
                        };
                        providers::sysutil::run_as_admin(&exe)
                    }
                    "reveal" => providers::sysutil::reveal_in_explorer(&resolved),
                    "copy_path" => providers::clipboard::retrying(|| {
                        let resolved = resolved.clone();
                        arboard::Clipboard::new().and_then(|mut cb| cb.set_text(resolved))
                    })
                    .map(|_| ()),
                    _ => Err("未知操作".into()),
                }
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn everything_setup() -> Result<String, String> {
    // 整个流程（下载/解压/提权启动/等待）都在阻塞线程池

    tauri::async_runtime::spawn_blocking(providers::everything::setup)
        .await
        .map_err(|e| e.to_string())?
}

/// 按开关取呼出前前台窗口（选中即粘贴的粘贴目标）；设置关闭或无记录
/// （呼出时前台就是面板/捕获失败）返回 None
fn take_prev_foreground(app: &AppHandle) -> Option<isize> {
    if providers::clipboard::PASTE_TO_PREV.load(Ordering::Relaxed) {
        let state = app.state::<AppState>();
        state
            .prev_foreground
            .lock()
            .expect("prev_foreground poisoned")
            .take()
    } else {
        None
    }
}

/// 后台回填文本到剪贴板；有呼出前前台则交还焦点并注入 Ctrl+V。
/// open_item 的 clip 文本分支与 clip_merge_paste 共用；回填会被自家监听
/// 收录进历史顶部（重复内容自动去重）。retrying 退避（最坏 ~370ms）与
/// 粘贴前的等待不能占主线程消息泵，压后台线程
fn paste_text_async(text: String, prev: Option<isize>) {
    std::thread::spawn(move || {
        let _ = providers::clipboard::retrying(|| {
            let text = text.clone();
            arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text))
        });
        if let Some(hwnd) = prev {
            std::thread::sleep(Duration::from_millis(80));
            providers::winsys::paste_to_prev(hwnd);
        }
    });
}

#[tauri::command]
fn open_item(app: AppHandle, item: SearchItem) -> Result<(), String> {
    // 剪贴板「选中即粘贴」在后台线程完成焦点交还与按键注入：open_item 在主线程，
    // 粘贴前的等待（焦点回落最长 ~600ms）不能压在这里
    let mut paste_started = false;
    let result = match item.kind.as_str() {
        "app" | "recent" | "file" | "stage" => {
            // 启动计数（frecency 排序依据）：仅应用类，按 target 路径累计
            if item.kind == "app" {
                app.state::<AppState>()
                    .engine
                    .record_app_launch(&item.target);
            }
            if item.target.starts_with("shell:") {
                // UWP/商店应用：explorer 接管 shell:AppsFolder\<AUMID>
                let _ = std::process::Command::new("explorer")
                    .arg(&item.target)
                    .spawn();
                Ok(())
            } else {
                app.opener().open_path(item.target.clone(), None::<&str>)
            }
        }
        "settings" => {
            // target 形如 ms-settings:xxx（协议 URI）或 control.exe / *.cpl / *.msc（经典系统工具）。
            // 带协议的交给 opener（ShellExecute 认 ms-settings:），裸文件名一律走
            // `cmd /C start "" <target>`：这是资源管理器"运行"同样的解析路径，
            // 能可靠启动系统可执行名与控制面板项，避免 ShellExecute 对裸名解析失败。
            // cmd 是控制台程序，spawn 走 spawn_hidden 防黑框闪现
            if item.target.contains(':') {
                if app
                    .opener()
                    .open_url(item.target.clone(), None::<&str>)
                    .is_err()
                {
                    let _ = spawn_hidden(std::process::Command::new("cmd").args([
                        "/C",
                        "start",
                        "",
                        &item.target,
                    ]));
                }
            } else {
                let _ = spawn_hidden(std::process::Command::new("cmd").args([
                    "/C",
                    "start",
                    "",
                    &item.target,
                ]));
            }
            Ok(())
        }
        "clip" | "clipimg" => {
            // 图片行号同步探存在：「已被清理」要即时回前端 toast（不收面板），不能异步化
            if item.kind == "clipimg" {
                let rowid = item
                    .target
                    .parse::<i64>()
                    .map_err(|_| "无效图片条目".to_string())?;
                if !app.state::<AppState>().engine.clip_image_exists(rowid) {
                    return Err("该条剪贴板图片已被清理".into());
                }
            }
            // 选中即粘贴：焦点交还呼出前的窗口并注入 Ctrl+V，省掉手动一步
            let prev = take_prev_foreground(&app);
            // 图片的点查全图 + PNG 解码不能占主线程消息泵，压后台线程；
            // 文本回填走共用 paste_text_async（内部自起线程）
            let engine = app.state::<AppState>().engine.clone();
            let target = item.target.clone();
            let is_img = item.kind == "clipimg";
            paste_started = prev.is_some();
            if prev.is_some() {
                hide_panel_window(&app);
            }
            if is_img {
                std::thread::spawn(move || {
                    // 图片=按行号点查全图并解码 RGBA 后回填。DIBV5 直写毫秒级，
                    // 注入前的等待才不会随图放大成秒级
                    if let Ok(rowid) = target.parse::<i64>()
                        && let Some((w, h, png)) = engine.clip_image(rowid)
                        && let Ok((_, _, rgba)) = providers::clipimage::decode_png(&png)
                    {
                        let _ = providers::clipboard::set_image_dib(w as usize, h as usize, &rgba);
                    }
                    if let Some(hwnd) = prev {
                        std::thread::sleep(Duration::from_millis(80));
                        providers::winsys::paste_to_prev(hwnd);
                    }
                });
            } else {
                paste_text_async(target, prev);
            }
            Ok(())
        }
        "win" => {
            if let Ok(hwnd) = item.target.parse::<isize>() {
                providers::winsys::activate(hwnd);
            }
            Ok(())
        }
        _ => app.opener().open_url(item.target.clone(), None::<&str>),
    };
    // 打开失败不收面板：错误回传前端 toast 提示，用户可另选条目
    if let Err(e) = result {
        return Err(e.to_string());
    }
    // stage 视图内打开不收面板（用户拍板）：中转站是「连续处理多个暂存项」
    // 的工作台心智——文件在原应用起来，面板留原位供继续点击/复制/固定，
    // Esc 或返回才收。其余 kind 维持启动即走
    if !paste_started && item.kind != "stage" {
        hide_panel_window(&app);
    }
    Ok(())
}

/// 剪贴板条目固定/取消固定（返回操作后的状态，前端据此刷新排序与按钮态）
#[tauri::command]
async fn clip_pin(state: tauri::State<'_, AppState>, id: i64) -> Result<bool, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.clip_pin(id))
        .await
        .map_err(|e| e.to_string())?
}

/// 图片条目固定/取消固定：直连图片表——clip_pin 的「文本表先命中」兜底链路
/// 在两表自增行号撞号时会固定到同号的文本行，图片条目必须按 kind 分流到此
#[tauri::command]
async fn clipimg_pin(state: tauri::State<'_, AppState>, id: i64) -> Result<bool, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.clip_image_pin(id))
        .await
        .map_err(|e| e.to_string())?
}

// ---------- 中转站 ----------

#[tauri::command]
async fn stage_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<providers::stage::StageRecord>, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.stage_list())
        .await
        .map_err(|e| e.to_string())
}

/// 批量放入：逐条尝试，个别失败不影响其余（失败明细回传前端提示）
#[tauri::command]
async fn stage_add(
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
) -> Result<providers::stage::StageAddOutcome, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.stage_add(&paths))
        .await
        .map_err(|e| e.to_string())
}

/// 移出中转站（只删引用，不碰原文件）
#[tauri::command]
async fn stage_remove(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.stage_remove(id))
        .await
        .map_err(|e| e.to_string())?
}

/// 批量移出（清理失效条目）：失效判定在前端列表数据里（list 时已逐条 exists），
/// 这里只按 id 删除，返回实际移出条数
#[tauri::command]
async fn stage_remove_batch(
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<usize, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.stage_remove_batch(&ids))
        .await
        .map_err(|e| e.to_string())
}

/// 清空临时区（固定条目保留），返回清掉条数
#[tauri::command]
async fn stage_clear(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.stage_clear())
        .await
        .map_err(|e| e.to_string())?
}

/// 固定/取消固定（返回操作后的状态）
#[tauri::command]
async fn stage_pin(state: tauri::State<'_, AppState>, id: i64) -> Result<bool, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.stage_pin(id))
        .await
        .map_err(|e| e.to_string())?
}

/// 重新定位：为失效条目重绑路径
#[tauri::command]
async fn stage_relocate(
    state: tauri::State<'_, AppState>,
    id: i64,
    path: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.stage_relocate(id, &path))
        .await
        .map_err(|e| e.to_string())?
}

/// 复制文件到剪贴板（CF_HDROP，可粘贴到任意位置）；无库访问但剪贴板写入带重试，照例压线程池
#[tauri::command]
async fn stage_copy_file(paths: Vec<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || providers::stage::copy_files_to_clipboard(&paths))
        .await
        .map_err(|e| e.to_string())?
}

/// 落区条当前反馈文案（前端挂载时拉取同步；None = 默认文案）。
/// drop 的接收与 stage_add 全在 Rust 侧，这是唯一的前端拉取点
#[tauri::command]
fn stagedrop_feedback_state() -> Option<String> {
    providers::stagedrop::current_text()
}

/// 落区 drop 的统一处理：自有 IDropTarget 主路径与 wry→on_window_event 兜底
/// 路径都汇到这里（同一窗口只会有一条路径存活，不会双触发）。
/// stage_add 压线程池——网络路径的 metadata 可达秒级，不能占调用线程
pub(crate) fn handle_stage_drop(app: &AppHandle, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    // drop 即呼出中转站面板（用户拍板 2026-10-28）：压独立线程一拍——自有
    // IDropTarget 路径本就在后台线程，wry 兜底路径的调用点在主线程事件循环
    // 的拖放事件里，同步开主面板会嵌进 OLE 拖拽循环收尾；面板与入库并行，
    // 列表靠 zap://stage-changed 刷新
    {
        let app = app.clone();
        std::thread::spawn(move || open_stage_panel(&app));
    }
    let generation = providers::stagedrop::begin_processing();
    // 可见性兜底：条子被收起后 Drop 才到达时补显
    providers::stagedrop::ensure_visible(app);
    let _ = app.emit_to(
        "stagedrop",
        "zap://stagedrop-feedback",
        providers::stagedrop::PROCESSING_TEXT.to_string(),
    );
    let engine = app.state::<AppState>().engine.clone();
    let app = app.to_owned();
    drop(tauri::async_runtime::spawn_blocking(move || {
        let outcome = engine.stage_add(&paths);
        let text = providers::stagedrop::feedback_text(&outcome);
        providers::stagedrop::finish_processing(
            generation,
            text.clone(),
            !outcome.failed.is_empty(),
        );
        let _ = app.emit_to("stagedrop", "zap://stagedrop-feedback", text);
        // 通知主窗口：中转站视图若开着（面板 hold 期间两条放入通道并存是常态），
        // 列表需要刷新；视图未开时无人订阅，事件落空无害
        let _ = app.emit("zap://stage-changed", ());
    }));
}

/// 批量撤销恢复：原样插回刚移出/清空的条目（含固定态与时间戳；路径已被
/// 重新放入的幂等跳过），返回恢复条数
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StageRestoreItem {
    path: String,
    is_dir: bool,
    pinned: bool,
    added_at: i64,
}

#[tauri::command]
async fn stage_restore_batch(
    state: tauri::State<'_, AppState>,
    items: Vec<StageRestoreItem>,
) -> Result<usize, String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.stage_restore_batch(
            items
                .into_iter()
                .map(|i| (i.path, i.is_dir, i.pinned, i.added_at))
                .collect(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 前台窗口的根窗口是否属于指定 hwnd（WebView2 子窗口夺焦时 tao 会误报
/// Focused(false)，焦点判定必须落在根窗口上）
#[cfg(windows)]
fn foreground_belongs_to_hwnd(ours: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetForegroundWindow};
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        GetAncestor(fg, GA_ROOT) == ours
    }
}

/// 前台窗口的根窗口是否是本面板（WebView2 子窗口夺焦时 tao 会误报 Focused(false)）
#[cfg(windows)]
fn foreground_belongs_to_panel(window: &tauri::Window) -> bool {
    let Ok(ours) = window.hwnd() else {
        return false;
    };
    foreground_belongs_to_hwnd(ours)
}

/// 呼出前的前台窗口（剪贴板「选中即粘贴」的焦点交还目标）。前台已是面板则不记
#[cfg(windows)]
fn capture_prev_foreground(window: &tauri::WebviewWindow) -> Option<isize> {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetForegroundWindow};
    let Ok(ours) = window.hwnd() else {
        return None;
    };
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() || GetAncestor(fg, GA_ROOT) == ours {
            return None;
        }
        Some(fg.0 as isize)
    }
}

#[cfg(not(windows))]
fn capture_prev_foreground(_window: &tauri::WebviewWindow) -> Option<isize> {
    None
}

/// 呼出/隐藏面板：显示时定位到鼠标附近（多显示器内夹紧），并通知前端清空状态。
/// preset 非空时在 shown 事件后追加 zap://preset（直达热键/超级面板进保护域用），
/// 前端在 shown 重置后再设置搜索词，顺序即生效顺序
pub(crate) fn toggle_panel(app: &AppHandle, preset: Option<&str>) {
    let Some(win) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        hide_panel_window(app);
        return;
    }
    let state = app.state::<AppState>();
    // 呼出即作废旧视图残留的 hold：中转站视图借用 hold 对抗失焦即隐，
    // 视图切换路径（如中转站→设置）可能漏释放，呼出时统一清零兜底
    state.hold_panel.store(false, Ordering::Relaxed);
    // 前台窗口必须在 show 之前记：show+set_focus 后前台就换成面板了
    *state
        .prev_foreground
        .lock()
        .expect("prev_foreground poisoned") = capture_prev_foreground(&win);
    // 显示时定位到光标所在显示器的正中央（窗口隐藏状态下移动不触发焦点事件）
    if let Ok(pos) = app.cursor_position()
        && let Ok(Some(monitor)) = app.monitor_from_point(pos.x, pos.y)
    {
        let mp = monitor.position();
        let ms = monitor.size();
        // 面板逻辑尺寸 × 该显示器缩放比 = 物理尺寸（否则高 DPI 下会偏右下）
        let scale = monitor.scale_factor();
        let pw = PANEL_WIDTH * scale;
        let ph = PANEL_HEIGHT * scale;
        let x = mp.x as f64 + (ms.width as f64 - pw) / 2.0;
        let y = mp.y as f64 + (ms.height as f64 - ph) / 2.0;
        let _ = win.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
    }
    let _ = win.show();
    let _ = win.set_focus();
    *state.shown_at.lock().expect("shown_at poisoned") = Some(Instant::now());
    let _ = win.emit("zap://shown", ());
    if let Some(p) = preset {
        let _ = app.emit("zap://preset", p.to_string());
    }
    // 面板呼出后台重建应用与最近文件索引（各自 10 秒节流）：新装/卸载的软件
    // 与刚打开过的文件无需重启即可搜到
    let engine = state.engine.clone();
    drop(tauri::async_runtime::spawn_blocking(move || {
        engine.refresh_apps();
        engine.refresh_recents();
    }));
}

/// drop 落区后立即呼出中转站面板（用户拍板 2026-10-28）：show 路径整体复用
/// toggle_panel（定位/prev_foreground/hold 清零/shown/preset 全继承），差异
/// 只在「可见即 no-op 不收」——toggle 语义会把已开的工作台收掉（评审 P1），
/// 而 drop 时刻面板可见的唯一常态就是 stage 视图 hold 中（拖拽第一步焦点
/// 就交给拖源，无 hold 的视图早被失焦即隐收掉），no-op 即正确。失败与否
/// 此刻未知，不做结果分流：面板与 stage_add 并行，列表靠 zap://stage-changed
/// 刷新；失败条目不进库，面板里表现为「没有新条目」，原因仍在条内反馈（3s）
pub(crate) fn open_stage_panel(app: &AppHandle) {
    if !providers::stagedrop::STAGEDROP_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Some(win) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        return;
    }
    toggle_panel(app, Some("stage"));
    // 焦点兜底（评审 P1）：drop 时刻前台是拖源应用，zap 是后台进程，
    // set_focus 可能被系统前台锁拒绝——面板停在「可见但无焦点」态（没有
    // Focused(false) 转变，失焦即隐不触发，正是失焦家族的 Esc 失灵形态）。
    // 延迟核对根窗口归属，确认没拿到前台则 Alt 解锁后重设一次
    #[cfg(windows)]
    {
        let win = win.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            let Ok(hwnd) = win.hwnd() else {
                return;
            };
            if foreground_belongs_to_hwnd(hwnd) {
                return;
            }
            let ok = providers::winsys::force_foreground(hwnd.0 as isize);
            providers::diag::log(&format!(
                "stagedrop: panel focus fallback force_foreground={ok}"
            ));
        });
    }
}

/// 直达热键·快速粘贴：不经面板，把最近复制的文本粘贴到当前前台窗口。
/// 剪贴板已有文本时直接注入（历史库不覆盖密码管理器来源/总开关关闭的复制，
/// 直接读库会把刚复制的内容覆盖成上一条历史）；空/非文本才回源历史库并回填。
/// 重活（剪贴板读写退避、SQLite 点查）压后台线程，不占热键回调所在的主线程
fn quick_paste_latest(app: &AppHandle) {
    // 前台窗口在回调内先取（无阻塞）；面板自身在前台时不注入（会贴进搜索框）
    let Some(hwnd) = foreground_hwnd() else {
        return;
    };
    if foreground_in_panel(app, hwnd) {
        return;
    }
    let engine = app.state::<AppState>().engine.clone();
    std::thread::spawn(move || {
        let current = arboard::Clipboard::new()
            .ok()
            .and_then(|mut cb| cb.get_text().ok());
        let (text, needs_backfill) = match current {
            Some(t) if !t.trim().is_empty() => (t, false),
            _ => match engine.latest_clip() {
                Some(t) => (t, true),
                None => return,
            },
        };
        if needs_backfill
            && providers::clipboard::retrying(|| {
                let text = text.clone();
                arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text))
            })
            .is_err()
        {
            return;
        }
        // 稍候再注入：热键组合的修饰键可能尚未松开
        std::thread::sleep(Duration::from_millis(80));
        providers::winsys::paste_to_prev(hwnd);
    });
}

/// 前台窗口是否属于本面板（快速粘贴防自注入：面板可见时按热键，前台就是面板）
#[cfg(windows)]
fn foreground_in_panel(app: &AppHandle, fg: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor};
    let Some(win) = app.get_webview_window(WINDOW_LABEL) else {
        return false;
    };
    let Ok(ours) = win.hwnd() else {
        return false;
    };
    unsafe { GetAncestor(HWND(fg as *mut core::ffi::c_void), GA_ROOT) == ours }
}

#[cfg(not(windows))]
fn foreground_in_panel(_app: &AppHandle, _fg: isize) -> bool {
    false
}

/// 当前前台窗口句柄（快速粘贴的注入目标）
#[cfg(windows)]
fn foreground_hwnd() -> Option<isize> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe {
        let fg = GetForegroundWindow();
        (!fg.is_invalid()).then_some(fg.0 as isize)
    }
}

#[cfg(not(windows))]
fn foreground_hwnd() -> Option<isize> {
    None
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 二次启动 = 呼出面板
            toggle_panel(app, None);
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(
            GlobalShortcutBuilder::new()
                .with_handler(|app, shortcut, event| {
                    // 插件 2.4 起按下与松开都会回调，只在按下时动作；
                    // 呼出热键可被用户改，按当前生效值匹配；其余查直达热键表
                    if event.state() == ShortcutState::Pressed {
                        let state = app.state::<AppState>();
                        let current: Option<Shortcut> =
                            state.hotkey.lock().expect("hotkey poisoned").parse().ok();
                        if current.as_ref() == Some(shortcut) {
                            toggle_panel(app, None);
                            return;
                        }
                        // 直达热键按「存档串 parse 成 Shortcut 再比较结构体」匹配，
                        // 与主呼出热键同一口径——不用 to_string() 做键：那是
                        // global-hotkey 的规范形（如 alt+KeyS），与存档串永不相等
                        let direct = state
                            .direct_hotkeys
                            .lock()
                            .expect("direct poisoned")
                            .iter()
                            .find(|(k, _)| k.parse::<Shortcut>().ok().as_ref() == Some(shortcut))
                            .map(|(_, v)| *v);
                        match direct {
                            Some(DirectKind::Vault) => toggle_panel(app, Some("pw")),
                            Some(DirectKind::QuickPaste) => quick_paste_latest(app),
                            Some(DirectKind::Stage) => toggle_panel(app, Some("stage")),
                            Some(DirectKind::SuperPanel) => {
                                // 抓取+识别+展示是重活（最长 ~400ms），压后台线程做
                                let handle = app.clone();
                                drop(tauri::async_runtime::spawn_blocking(move || {
                                    providers::superpanel::trigger_from_hotkey(&handle)
                                }));
                            }
                            None => {
                                eprintln!("zap: 收到热键事件但不匹配当前设置（当前 {current:?}）")
                            }
                        }
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .on_window_event(|window, event| {
            // 中转站拖拽落区：wry→tauri DragDrop 路径作为自有 IDropTarget 的
            // 兜底（主路径在 stagedrop.rs 的 StageDropTarget，替换了 wry 在该
            // 窗口的注册；两条路互斥不会双触发）
            if window.label() == "stagedrop"
                && let WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event
            {
                let paths: Vec<String> = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                handle_stage_drop(window.app_handle(), paths);
            }
            // 失焦即隐："毫无存在感"的核心；显示后 300ms 内的焦点抖动忽略
            if let WindowEvent::Focused(false) = event {
                if window.label() == "superpanel" {
                    // 超级面板：点空白/Esc 前焦点先掉，失焦即收
                    providers::superpanel::superpanel_hide(window.app_handle());
                    return;
                }
                if window.label() == WINDOW_LABEL {
                    let state = window.app_handle().state::<AppState>();
                    if state.hold_panel.load(std::sync::atomic::Ordering::Relaxed) {
                        return; // 安装流程期间保持面板可见（UAC 会抢焦点）
                    }
                    let within_grace = state
                        .shown_at
                        .lock()
                        .expect("shown_at poisoned")
                        .is_some_and(|t| t.elapsed() < Duration::from_millis(300));
                    let fg_in_panel = foreground_belongs_to_panel(window);
                    if !within_grace && !fg_in_panel {
                        hide_panel_window(window.app_handle());
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            search,
            hide_panel,
            open_item,
            clip_pin,
            stage_list,
            stage_add,
            stage_remove,
            stage_remove_batch,
            stage_clear,
            stage_pin,
            stage_relocate,
            stage_restore_batch,
            stage_copy_file,
            stagedrop_feedback_state,
            everything_status,
            everything_ensure,
            everything_setup,
            set_hold_panel,
            vault_search,
            vault_add,
            vault_action,
            vault_get,
            vault_update,
            vault_parse,
            vault_import,
            vault_backup_export,
            vault_backup_decrypt,
            vault_backup_restore,
            secondary_action,
            json_analyze,
            json_format,
            json_to_yaml,
            json_to_ts,
            tool_convert_all,
            tool_uuid,
            clip_row,
            clip_merge_paste,
            clipimg_preview,
            clipimg_copy,
            clipimg_export,
            clipimg_pin,
            copy_text,
            shortcuts_list,
            shortcuts_save,
            get_app_settings,
            save_settings,
            autostart_set,
            config_export,
            config_import,
            transparency_set,
            superpanel_act,
            superpanel_hide,
            superpanel_payload,
            start_hotkey_capture,
            stop_hotkey_capture
        ])
        .setup(|app| {
            // 数据库与剪贴板监听线程
            let db_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&db_dir)?;
            providers::diag::init(db_dir.join("zap.log"));
            let db_path = db_dir.join("zap.db");
            let engine = SearchEngine::new(&db_path);
            providers::clipboard::spawn_watcher(db_path);

            // 应用设置：静态量 + 热键（存档失效回退默认；都失败则仅托盘可呼出）
            let clip_enabled = engine
                .get_setting(providers::appconfig::KEY_CLIP_ENABLED)
                .map(|v| v != "0")
                .unwrap_or(true);
            providers::clipboard::CLIP_ENABLED.store(clip_enabled, Ordering::Relaxed);
            let clip_max: i64 = engine
                .get_setting(providers::appconfig::KEY_CLIP_MAX)
                .and_then(|v| v.parse().ok())
                .unwrap_or(providers::appconfig::DEFAULT_CLIP_MAX);
            providers::clipboard::RETENTION_LIMIT.store(clip_max, Ordering::Relaxed);
            let clip_autopaste = engine
                .get_setting(providers::appconfig::KEY_CLIP_AUTOPASTE)
                .map(|v| v != "0")
                .unwrap_or(providers::appconfig::DEFAULT_CLIP_AUTOPASTE);
            providers::clipboard::PASTE_TO_PREV.store(clip_autopaste, Ordering::Relaxed);

            // 中转站容量：存档失效回退默认（10–500 之外视为坏值）
            let stage_cap: i64 = engine
                .get_setting(providers::appconfig::KEY_STAGE_CAP)
                .and_then(|v| v.parse().ok())
                .filter(|v| (10..=500).contains(v))
                .unwrap_or(providers::appconfig::DEFAULT_STAGE_CAP);
            providers::stage::CAP.store(stage_cap, Ordering::Relaxed);

            // 拖拽落区条：检测线程无条件启动（互验 P1——冷启动关闭态下运行时
            // 重开也要即时生效），线程内按存档开关门控（关闭态仅空闲探测）
            let stagedrop_on = engine
                .get_setting(providers::appconfig::KEY_STAGEDROP_ENABLED)
                .map(|v| v != "0")
                .unwrap_or(providers::appconfig::DEFAULT_STAGEDROP_ENABLED);
            providers::stagedrop::STAGEDROP_ENABLED.store(stagedrop_on, Ordering::Relaxed);
            providers::stagedrop::spawn(app.handle().clone());
            providers::stagedrop::register_drop_target(app.handle());

            let saved_hotkey = engine
                .get_setting(providers::appconfig::KEY_HOTKEY)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| providers::appconfig::DEFAULT_HOTKEY.into());
            let gs = app.global_shortcut();
            let mut active_hotkey: Option<String> = None;
            for cand in [
                saved_hotkey.clone(),
                providers::appconfig::DEFAULT_HOTKEY.to_string(),
            ] {
                if let Ok(sc) = cand.parse::<Shortcut>()
                    && gs.register(sc).is_ok()
                {
                    active_hotkey = Some(cand);
                    break;
                }
            }
            let registered_ok = active_hotkey.is_some();
            let hotkey_now =
                active_hotkey.unwrap_or_else(|| providers::appconfig::DEFAULT_HOTKEY.into());
            if !registered_ok {
                eprintln!("zap: 呼出热键注册失败，仅可通过托盘呼出面板");
            }

            // 直达热键：账号库 / 快速粘贴 / 超级面板 / 中转站
            // （未绑定 = 空串；注册失败仅落日志不影响启动）
            let mut direct = HashMap::new();
            for (cfg_key, kind) in [
                (providers::appconfig::KEY_VAULT_HOTKEY, DirectKind::Vault),
                (
                    providers::appconfig::KEY_CLIP_HOTKEY,
                    DirectKind::QuickPaste,
                ),
                (
                    providers::appconfig::KEY_SUPERPANEL_HOTKEY,
                    DirectKind::SuperPanel,
                ),
                (providers::appconfig::KEY_STAGE_HOTKEY, DirectKind::Stage),
            ] {
                let hk = engine
                    .get_setting(cfg_key)
                    .unwrap_or_default()
                    .trim()
                    .to_lowercase();
                if hk.is_empty() {
                    continue;
                }
                match hk.parse::<Shortcut>() {
                    Ok(sc) if gs.register(sc).is_ok() => {
                        direct.insert(hk, kind);
                    }
                    _ => eprintln!("zap: 直达热键 {hk} 注册失败"),
                }
            }

            // 透明面板（亚克力）：按存档偏好应用窗口特效；系统不支持仅落日志，
            // 面板表面保持不透明（前端 zap-trans 类与该偏好同步）
            let transparency_on = engine
                .get_setting(providers::appconfig::KEY_PANEL_TRANSPARENCY)
                .map(|v| v != "0")
                .unwrap_or(providers::appconfig::DEFAULT_PANEL_TRANSPARENCY);
            if let Err(e) = apply_window_effects(app.handle(), transparency_on) {
                eprintln!("zap: 透明效果应用失败（面板保持不透明）: {e}");
                // 偏好落 0 收敛状态：否则前端按存档偏好挂 zap-trans 类，
                // 得到无模糊底的纯半透明面板（设置页路径失败本就不落偏好，此处对齐）
                if transparency_on {
                    let _ = engine.set_setting(providers::appconfig::KEY_PANEL_TRANSPARENCY, "0");
                }
            }

            // Everything 自动检测：未运行且已安装则自动拉起（UAC 会弹一次）。
            // 未安装不自动下载——面板里仍保留手动入口。设置里可关掉。
            let ev_auto = engine
                .get_setting(providers::appconfig::KEY_EVERYTHING_AUTOSTART)
                .map(|v| v != "0")
                .unwrap_or(providers::appconfig::DEFAULT_EVERYTHING_AUTOSTART);
            if ev_auto {
                // 稍等桌面就绪再探测启动，避免开机自启时与登录过程抢占
                drop(tauri::async_runtime::spawn_blocking(|| {
                    std::thread::sleep(Duration::from_secs(3));
                    providers::everything::ensure()
                }));
            }

            app.manage(AppState {
                engine: std::sync::Arc::new(engine),
                hotkey: Mutex::new(hotkey_now),
                direct_hotkeys: Mutex::new(direct),
                clip_item: Mutex::new(None),
                shown_at: Mutex::new(None),
                hold_panel: AtomicBool::new(false),
                prev_foreground: Mutex::new(None),
            });

            // 托盘：左键单击 = 呼出面板（满足"桌面点击一下"）
            let show = MenuItem::with_id(app, "show", "显示面板", true, None::<&str>)?;
            let open_settings =
                MenuItem::with_id(app, "open-settings", "设置…", true, None::<&str>)?;
            let clip_toggle = CheckMenuItem::with_id(
                app,
                "clip-toggle",
                "记录剪贴板历史",
                true,
                clip_enabled,
                None::<&str>,
            )?;
            let autostart_item = CheckMenuItem::with_id(
                app,
                "autostart",
                "开机自启",
                true,
                app.autolaunch().is_enabled().unwrap_or(false),
                None::<&str>,
            )?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "退出 zap", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &show,
                    &open_settings,
                    &sep,
                    &clip_toggle,
                    &autostart_item,
                    &sep,
                    &quit,
                ],
            )?;
            *app.state::<AppState>()
                .clip_item
                .lock()
                .expect("clip_item poisoned") = Some(clip_toggle.clone());
            TrayIconBuilder::with_id("zap-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("zap — 个人效率入口")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => toggle_panel(app, None),
                    "open-settings" => {
                        let visible = app
                            .get_webview_window(WINDOW_LABEL)
                            .map(|w| w.is_visible().unwrap_or(false))
                            .unwrap_or(false);
                        if !visible {
                            toggle_panel(app, None);
                        }
                        let _ = app.emit("zap://open-settings", ());
                    }
                    "clip-toggle" => {
                        let now = !providers::clipboard::CLIP_ENABLED.load(Ordering::Relaxed);
                        providers::clipboard::CLIP_ENABLED.store(now, Ordering::Relaxed);
                        let st = app.state::<AppState>();
                        // 显式回写勾选态：不押注 muda 在 MenuEvent 前自动翻转
                        // （各平台行为不一），与 save_settings 路径的 set_checked 同口径
                        if let Some(item) =
                            st.clip_item.lock().expect("clip_item poisoned").as_ref()
                        {
                            let _ = item.set_checked(now);
                        }
                        let _ = st.engine.set_setting(
                            providers::appconfig::KEY_CLIP_ENABLED,
                            if now { "1" } else { "0" },
                        );
                    }
                    "autostart" => {
                        if app.autolaunch().is_enabled().unwrap_or(false) {
                            let _ = app.autolaunch().disable();
                        } else {
                            let _ = app.autolaunch().enable();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_panel(tray.app_handle(), None);
                    }
                })
                .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
