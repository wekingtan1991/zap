mod hotkey_capture;
mod providers;

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

struct AppState {
    engine: std::sync::Arc<SearchEngine>,
    /// 当前生效的呼出热键（设置保存时热切换）
    hotkey: Mutex<String>,
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
    clip_enabled: bool,
    clip_max: i64,
    /// 剪贴板条目选中后自动粘贴到原窗口
    clip_autopaste: bool,
    pwd_clear_secs: i64,
    autostart: bool,
    everything_autostart: bool,
    /// 可开关插件清单（仅工具插件；核心搜索源不列）
    plugins: Vec<providers::PluginInfo>,
}

#[tauri::command]
fn get_app_settings(app: AppHandle, state: tauri::State<AppState>) -> AppSettingsDto {
    AppSettingsDto {
        hotkey: state.hotkey.lock().expect("hotkey poisoned").clone(),
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
}

// Tauri 命令按字段平铺（前端 invoke 传同名参数），显式放行参数个数提示
#[allow(clippy::too_many_arguments)]
#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: tauri::State<AppState>,
    hotkey: String,
    clip_enabled: bool,
    clip_max: i64,
    // Option 容错：半新半旧的会话（前端未带上新键）保存时不再整单被拒
    clip_autopaste: Option<bool>,
    pwd_clear_secs: i64,
    everything_autostart: bool,
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
                    *state.hotkey.lock().expect("hotkey poisoned") = hotkey;
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
    Ok(SaveOutcome { hotkey_error })
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
/// 走自身复制抑制窗，派生结果不重复进入剪贴板历史
#[tauri::command]
async fn copy_text(text: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        providers::clipboard::begin_self_copy();
        arboard::Clipboard::new()
            .map_err(|e| e.to_string())?
            .set_text(text)
            .map_err(|e| e.to_string())?;
        Ok("已复制".to_string())
    })
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

/// 条目次级操作：admin=管理员运行 / reveal=所在文件夹定位 / copy_path=复制路径。
/// .lnk 先解析为真实目标（要的是 exe 而不是开始菜单快捷方式）；商店应用（shell:）不支持
#[tauri::command]
async fn secondary_action(
    state: tauri::State<'_, AppState>,
    action: String,
    target: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
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
            "copy_path" => arboard::Clipboard::new()
                .map_err(|e| e.to_string())?
                .set_text(resolved)
                .map_err(|e| e.to_string()),
            _ => Err("未知操作".into()),
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
        "clip" => {
            // 回填剪贴板；我们的监听会把它移动到历史顶部（重复内容自动去重）
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(item.target.clone());
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
            if let Some(hwnd) = prev {
                hide_panel_window(&app);
                paste_started = true;
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(80));
                    providers::winsys::paste_to_prev(hwnd);
                });
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

/// 呼出/隐藏面板：显示时定位到鼠标附近（多显示器内夹紧），并通知前端清空状态
fn toggle_panel(app: &AppHandle) {
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
    // 面板呼出后台重建应用索引（10 秒节流）：新装/卸载的软件无需重启即可搜到
    let engine = state.engine.clone();
    drop(tauri::async_runtime::spawn_blocking(move || {
        engine.refresh_apps()
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 二次启动 = 呼出面板
            toggle_panel(app);
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(
            GlobalShortcutBuilder::new()
                .with_handler(|app, shortcut, event| {
                    // 插件 2.4 起按下与松开都会回调，只在按下时切换面板；
                    // 热键可被用户改，按当前生效值匹配
                    if event.state() == ShortcutState::Pressed {
                        let state = app.state::<AppState>();
                        let current: Option<Shortcut> =
                            state.hotkey.lock().expect("hotkey poisoned").parse().ok();
                        if current.as_ref() == Some(shortcut) {
                            toggle_panel(app);
                        } else {
                            eprintln!("zap: 收到热键事件但不匹配当前设置（当前 {current:?}）");
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
            if let WindowEvent::Focused(false) = event
                && window.label() == WINDOW_LABEL
            {
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
            clip_row,
            copy_text,
            get_app_settings,
            save_settings,
            autostart_set,
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
                    "show" => toggle_panel(app),
                    "open-settings" => {
                        let visible = app
                            .get_webview_window(WINDOW_LABEL)
                            .map(|w| w.is_visible().unwrap_or(false))
                            .unwrap_or(false);
                        if !visible {
                            toggle_panel(app);
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
                        toggle_panel(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
