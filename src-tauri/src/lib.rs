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

#[tauri::command]
fn everything_status() -> String {
    providers::everything::status()
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
fn vault_search(state: tauri::State<AppState>, query: String) -> Vec<SearchItem> {
    state.engine.vault_search(&query)
}

#[tauri::command]
fn vault_add(
    state: tauri::State<AppState>,
    title: String,
    username: String,
    password: String,
    url: String,
    notes: String,
) -> Result<(), String> {
    state
        .engine
        .vault_add(&title, &username, &password, &url, &notes)
}

#[tauri::command]
fn vault_action(state: tauri::State<AppState>, id: i64, action: String) -> Result<String, String> {
    let secs: i64 = state
        .engine
        .get_setting(providers::appconfig::KEY_PWD_CLEAR_SECS)
        .and_then(|v| v.parse().ok())
        .unwrap_or(providers::appconfig::DEFAULT_PWD_CLEAR_SECS);
    state.engine.vault_action(id, &action, secs)
}

/// 取单条账号完整记录（编辑预填；含解密后的密码明文）
#[tauri::command]
fn vault_get(
    state: tauri::State<AppState>,
    id: i64,
) -> Result<(String, String, String, String, String, i64), String> {
    state.engine.vault_get(id)
}

// Tauri 命令按字段平铺（前端 invoke 传同名参数），显式放行参数个数提示
#[allow(clippy::too_many_arguments)]
#[tauri::command]
fn vault_update(
    state: tauri::State<AppState>,
    id: i64,
    title: String,
    username: String,
    password: String,
    url: String,
    notes: String,
    pinned: i64,
) -> Result<(), String> {
    state
        .engine
        .vault_update(id, &title, &username, &password, &url, &notes, pinned)
}

// ---------- 应用设置 ----------

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsDto {
    hotkey: String,
    clip_enabled: bool,
    clip_max: i64,
    pwd_clear_secs: i64,
    autostart: bool,
    everything_autostart: bool,
}

#[tauri::command]
fn get_app_settings(app: AppHandle, state: tauri::State<AppState>) -> AppSettingsDto {
    AppSettingsDto {
        hotkey: state.hotkey.lock().expect("hotkey poisoned").clone(),
        clip_enabled: providers::clipboard::CLIP_ENABLED.load(Ordering::Relaxed),
        clip_max: providers::clipboard::RETENTION_LIMIT.load(Ordering::Relaxed),
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
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveOutcome {
    /// 热键保存失败的原因；None = 生效
    hotkey_error: Option<String>,
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: tauri::State<AppState>,
    hotkey: String,
    clip_enabled: bool,
    clip_max: i64,
    pwd_clear_secs: i64,
    everything_autostart: bool,
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
    state.engine.set_setting(
        providers::appconfig::KEY_PWD_CLEAR_SECS,
        &pwd_clear_secs.to_string(),
    )?;
    state.engine.set_setting(
        providers::appconfig::KEY_EVERYTHING_AUTOSTART,
        if everything_autostart { "1" } else { "0" },
    )?;
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
fn vault_parse(text: String) -> Vec<providers::vault::ParsedAccount> {
    providers::vault::parse_blob(&text)
}

#[tauri::command]
fn vault_import(
    state: tauri::State<AppState>,
    entries: Vec<providers::vault::ParsedAccount>,
) -> Result<usize, String> {
    state.engine.vault_import(entries)
}

#[tauri::command]
async fn everything_setup() -> Result<String, String> {
    // 整个流程（下载/解压/提权启动/等待）都在阻塞线程池

    tauri::async_runtime::spawn_blocking(providers::everything::setup)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn open_item(app: AppHandle, item: SearchItem) {
    let result = match item.kind.as_str() {
        "app" | "recent" | "file" => {
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
            if item.target.contains(':') {
                if app
                    .opener()
                    .open_url(item.target.clone(), None::<&str>)
                    .is_err()
                {
                    let _ = std::process::Command::new("cmd")
                        .args(["/C", "start", "", &item.target])
                        .spawn();
                }
            } else {
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", "", &item.target])
                    .spawn();
            }
            Ok(())
        }
        "clip" => {
            // 回填剪贴板；我们的监听会把它移动到历史顶部（重复内容自动去重）
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(item.target.clone());
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
    if let Err(e) = result {
        eprintln!("zap: 打开失败: {e}");
    }
    hide_panel_window(&app);
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

/// 呼出/隐藏面板：显示时定位到鼠标附近（多显示器内夹紧），并通知前端清空状态
fn toggle_panel(app: &AppHandle) {
    let Some(win) = app.get_webview_window(WINDOW_LABEL) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        hide_panel_window(app);
        return;
    }
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
    let state = app.state::<AppState>();
    *state.shown_at.lock().expect("shown_at poisoned") = Some(Instant::now());
    let _ = win.emit("zap://shown", ());
    // 面板呼出即后台重建应用索引：新装/卸载的软件无需重启即可搜到（防并发叠加）
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
