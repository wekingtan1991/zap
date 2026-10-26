mod hotkey_capture;
mod providers;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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
/// 把最近一条剪贴板内容粘贴到当前前台窗口；超级面板 = 抓取选中文本出动作菜单
#[derive(Clone, Copy)]
enum DirectKind {
    Vault,
    QuickPaste,
    SuperPanel,
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
    clip_enabled: bool,
    clip_max: i64,
    /// 剪贴板条目选中后自动粘贴到原窗口
    clip_autopaste: bool,
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

#[tauri::command]
fn get_app_settings(app: AppHandle, state: tauri::State<AppState>) -> AppSettingsDto {
    AppSettingsDto {
        hotkey: state.hotkey.lock().expect("hotkey poisoned").clone(),
        vault_hotkey: state
            .engine
            .get_setting(providers::appconfig::KEY_VAULT_HOTKEY)
            .unwrap_or_default(),
        clip_hotkey: state
            .engine
            .get_setting(providers::appconfig::KEY_CLIP_HOTKEY)
            .unwrap_or_default(),
        superpanel_hotkey: state
            .engine
            .get_setting(providers::appconfig::KEY_SUPERPANEL_HOTKEY)
            .unwrap_or_default(),
        clip_enabled: providers::clipboard::CLIP_ENABLED.load(Ordering::Relaxed),
        clip_max: providers::clipboard::RETENTION_LIMIT.load(Ordering::Relaxed),
        clip_autopaste: providers::clipboard::PASTE_TO_PREV.load(Ordering::Relaxed),
        pwd_clear_secs: state
            .engine
            .get_setting(providers::appconfig::KEY_PWD_CLEAR_SECS)
            .and_then(|v| v.parse().ok())
            .unwrap_or(providers::appconfig::DEFAULT_PWD_CLEAR_SECS),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        everything_autostart: state
            .engine
            .get_setting(providers::appconfig::KEY_EVERYTHING_AUTOSTART)
            .map(|v| v != "0")
            .unwrap_or(providers::appconfig::DEFAULT_EVERYTHING_AUTOSTART),
        appearance: state
            .engine
            .get_setting(providers::appconfig::KEY_APPEARANCE)
            .unwrap_or_else(|| "dark".into()),
        panel_transparency: state
            .engine
            .get_setting(providers::appconfig::KEY_PANEL_TRANSPARENCY)
            .map(|v| v != "0")
            .unwrap_or(providers::appconfig::DEFAULT_PANEL_TRANSPARENCY),
        plugins: state.engine.plugin_states(),
    }
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

// Tauri 命令按字段平铺（前端 invoke 传同名参数），显式放行参数个数提示
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
    clip_enabled: bool,
    clip_max: i64,
    clip_autopaste: Option<bool>,
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
        for (raw, cfg_key, kind) in [
            (
                vault_hotkey,
                providers::appconfig::KEY_VAULT_HOTKEY,
                DirectKind::Vault,
            ),
            (
                clip_hotkey,
                providers::appconfig::KEY_CLIP_HOTKEY,
                DirectKind::QuickPaste,
            ),
            (
                superpanel_hotkey,
                providers::appconfig::KEY_SUPERPANEL_HOTKEY,
                DirectKind::SuperPanel,
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
                direct_errors.push("直达热键不能与呼出热键相同，各直达热键之间也不能相同".into());
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
                        direct_errors.push(format!("直达热键注册失败，可能已被其他程序占用：{e}"));
                        // 恢复旧绑定
                        if let Ok(old_sc) = old.parse::<Shortcut>() {
                            let _ = gs.register(old_sc);
                        }
                    }
                },
                Err(e) => {
                    direct_errors.push(format!("无法识别的按键组合：{e}"));
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

#[tauri::command]
fn open_item(app: AppHandle, item: SearchItem) -> Result<(), String> {
    // 剪贴板「选中即粘贴」在后台线程完成焦点交还与按键注入：open_item 在主线程，
    // 粘贴前的等待（焦点回落最长 ~600ms）不能压在这里
    let mut paste_started = false;
    let result = match item.kind.as_str() {
        "app" | "recent" | "file" => {
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
            // 选中即粘贴：焦点交还呼出前的窗口并注入 Ctrl+V，省掉手动一步；
            // 设置关闭或无记录（呼出时前台就是面板/捕获失败）则退化为仅回填
            let prev = if providers::clipboard::PASTE_TO_PREV.load(Ordering::Relaxed) {
                let state = app.state::<AppState>();
                state
                    .prev_foreground
                    .lock()
                    .expect("prev_foreground poisoned")
                    .take()
            } else {
                None
            };
            // 回填与粘贴压后台线程：retrying 退避（最坏 ~370ms）与图片的
            // 点查全图 + PNG 解码不能占主线程消息泵
            let engine = app.state::<AppState>().engine.clone();
            let target = item.target.clone();
            let is_img = item.kind == "clipimg";
            paste_started = prev.is_some();
            if prev.is_some() {
                hide_panel_window(&app);
            }
            std::thread::spawn(move || {
                // 回填剪贴板（文本=内容本体；图片=按行号点查全图并解码 RGBA）；
                // 我们的监听会把它移动到历史顶部（重复内容自动去重）
                if is_img {
                    if let Ok(rowid) = target.parse::<i64>()
                        && let Some((w, h, png)) = engine.clip_image(rowid)
                        && let Ok((_, _, rgba)) = providers::clipimage::decode_png(&png)
                    {
                        let _ = providers::clipboard::retrying(|| {
                            let rgba = rgba.clone();
                            arboard::Clipboard::new().and_then(|mut cb| {
                                cb.set_image(arboard::ImageData {
                                    width: w as usize,
                                    height: h as usize,
                                    bytes: std::borrow::Cow::Owned(rgba),
                                })
                            })
                        });
                    }
                } else {
                    let _ = providers::clipboard::retrying(|| {
                        let target = target.clone();
                        arboard::Clipboard::new().and_then(|mut cb| cb.set_text(target))
                    });
                }
                if let Some(hwnd) = prev {
                    std::thread::sleep(Duration::from_millis(80));
                    providers::winsys::paste_to_prev(hwnd);
                }
            });
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
    if !paste_started {
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

/// 前台窗口的根窗口是否是本面板（WebView2 子窗口夺焦时 tao 会误报 Focused(false)）
#[cfg(windows)]
fn foreground_belongs_to_panel(window: &tauri::Window) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, GetForegroundWindow};
    let Ok(ours) = window.hwnd() else {
        return false;
    };
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        GetAncestor(fg, GA_ROOT) == ours
    }
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
    // 面板呼出后台重建应用索引（10 秒节流）：新装/卸载的软件无需重启即可搜到
    let engine = state.engine.clone();
    drop(tauri::async_runtime::spawn_blocking(move || {
        engine.refresh_apps()
    }));
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
            copy_text,
            shortcuts_list,
            shortcuts_save,
            get_app_settings,
            save_settings,
            autostart_set,
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

            // 直达热键：账号库 / 快速粘贴 / 超级面板
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
