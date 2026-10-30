mod commands;
mod hotkey_capture;
mod panel;
mod providers;
mod settings;
mod state;

use panel::{handle_stage_drop, hide_panel_window, quick_paste_latest, toggle_panel};
use providers::SearchEngine;
use state::{AppState, DirectKind, WINDOW_LABEL};

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{
    Emitter, Manager, WindowEvent,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{
    Builder as GlobalShortcutBuilder, GlobalShortcutExt, Shortcut, ShortcutState,
};

// 应用状态与共享常量已拆至 state.rs（评审 P1-17 步 2）：AppState/DirectKind/
// WINDOW_LABEL/PANEL_WIDTH/PANEL_HEIGHT/spawn_hidden 经顶部 use 引入，lib.rs
// 内部引用面零改动

// 命令层已拆至 commands.rs（评审 P1-17 步 4）：search/everything/vault/
// clip/clipimg/stage/json/tool/superpanel/备份/secondary_action/open_item
// 等约 50 个命令。命令名与线协议不变，invoke_handler 引用 commands::*。
// 除轻量开关外命令一律 async + spawn_blocking：Tauri 同步命令跑主线程，
// SQLite 与剪贴板重试窗口会卡住消息泵
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // panic hook 最先挂：panic="abort" 下任何一处 panic 直接终止进程，release
    // 的 windows_subsystem 连 stderr 都不可见——这条 hook 是唯一的死因留痕
    // （P0-1 阶段0）。hook 内只做无锁操作（diag 的 LOG_PATH 是 OnceLock、
    // 文件 append 不持锁）：持锁线程 panic 时 hook 不再碰任何锁，杜绝 hook
    // 自死锁；diag::init 之前的极早期崩溃由 log_panic 落临时目录兜底。
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown payload".into()
        };
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        providers::diag::log_panic(&format!("panic\t{payload}\t{loc}"));
        eprintln!("zap panic: {payload} at {loc}");
    }));
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
                        let current: Option<Shortcut> = state
                            .hotkey
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .parse()
                            .ok();
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
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
                                providers::diag::log_warn(&format!(
                                    "收到热键事件但不匹配当前设置（当前 {current:?}）"
                                ))
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
            // 失焦即隐："毫无存在感"的核心。决策提纯进 panel::should_hide_on_blur
            // （纯函数+三场景单测，评审 P1-17 步 3）：hold 保活/300ms 抖动宽限/
            // 前台仍在面板 任一成立不收
            if let WindowEvent::Focused(false) = event {
                if window.label() == "superpanel" {
                    // 超级面板：点空白/Esc 前焦点先掉，失焦即收
                    providers::superpanel::superpanel_hide(window.app_handle());
                    return;
                }
                if window.label() == WINDOW_LABEL {
                    let state = window.app_handle().state::<AppState>();
                    let hold = state.hold_panel.load(std::sync::atomic::Ordering::Relaxed);
                    let within_grace = state
                        .shown_at
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_some_and(|t| t.elapsed() < Duration::from_millis(300));
                    let fg_in_panel = panel::foreground_belongs_to_panel(window);
                    if panel::should_hide_on_blur(hold, within_grace, fg_in_panel) {
                        hide_panel_window(window.app_handle());
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::search,
            panel::hide_panel,
            commands::open_item,
            commands::clip_pin,
            commands::stage_list,
            commands::stage_add,
            commands::stage_remove,
            commands::stage_remove_batch,
            commands::stage_clear,
            commands::stage_pin,
            commands::stage_relocate,
            commands::stage_restore_batch,
            commands::stage_copy_file,
            commands::stagedrop_feedback_state,
            commands::everything_status,
            commands::everything_ensure,
            commands::everything_setup,
            panel::set_hold_panel,
            panel::set_stage_view,
            commands::vault_search,
            commands::vault_add,
            commands::vault_action,
            commands::vault_get,
            commands::vault_update,
            commands::vault_parse,
            commands::vault_import,
            commands::vault_backup_export,
            commands::vault_backup_decrypt,
            commands::vault_backup_restore,
            commands::vault_last_export_at,
            commands::clip_autopaste_enabled,
            commands::secondary_action,
            commands::json_analyze_full,
            commands::json_to_yaml,
            commands::json_to_ts,
            commands::tool_convert_all,
            commands::tool_uuid,
            commands::clip_row,
            commands::clip_delete,
            commands::clip_clear_unpinned,
            commands::clip_merge_paste,
            commands::clipimg_preview,
            commands::clipimg_copy,
            commands::clipimg_export,
            commands::clipimg_pin,
            commands::copy_text,
            commands::shortcuts_list,
            commands::shortcuts_save,
            settings::get_app_settings,
            settings::save_settings,
            commands::autostart_set,
            settings::config_export,
            settings::config_import,
            settings::transparency_set,
            commands::superpanel_act,
            commands::superpanel_hide,
            commands::superpanel_payload,
            commands::start_hotkey_capture,
            commands::stop_hotkey_capture,
            commands::open_data_dir
        ])
        .setup(|app| {
            // 数据库与剪贴板监听线程
            let db_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&db_dir)?;
            providers::diag::init(db_dir.join("zap.log"));
            let db_path = db_dir.join("zap.db");
            let engine = SearchEngine::new(&db_path);
            providers::clipboard::spawn_watcher(db_path);

            // providers → crate 根的反向依赖边以回调注册反转（评审 P1-17 步 0，
            // lib.rs 拆分的前置）：drop 编排与「在 zap 中打开」属于面板编排层，
            // 留在 crate 根；providers 只持回调引用
            providers::stagedrop::register_drop_handler(std::sync::Arc::new(handle_stage_drop));
            providers::superpanel::register_open_panel(std::sync::Arc::new(|app, text| {
                toggle_panel(app, Some(&text));
            }));

            // 应用设置装载（owner 单一来源，评审 P1-18）：存档失效回退默认；
            // 「读键→解析→原子量」的镜像不再散落本文件
            let clip = engine.load_clip_settings();
            let clip_enabled = clip.enabled;
            engine.load_stage_cap();
            engine.load_stagedrop_settings();

            // 拖拽落区条：检测线程无条件启动（互验 P1——冷启动关闭态下运行时
            // 重开也要即时生效），线程内按存档开关门控（关闭态仅空闲探测）
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
                providers::diag::log_err("呼出热键注册失败（含默认键兜底），仅托盘可呼出");
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
                    _ => providers::diag::log_warn(&format!("直达热键 {hk} 注册失败")),
                }
            }

            // 透明面板（亚克力）：按存档偏好应用窗口特效；系统不支持仅落日志，
            // 面板表面保持不透明（前端 zap-trans 类与该偏好同步）
            let transparency_on = engine
                .get_setting(providers::appconfig::KEY_PANEL_TRANSPARENCY)
                .map(|v| v != "0")
                .unwrap_or(providers::appconfig::DEFAULT_PANEL_TRANSPARENCY);
            if let Err(e) = settings::apply_window_effects(app.handle(), transparency_on) {
                providers::diag::log_warn(&format!("透明效果应用失败（面板保持不透明）: {e}"));
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
                hotkey: Mutex::new(hotkey_now.clone()),
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
            // 托盘 tooltip 是呼出热键在产品内最大的静态露出面（评审 P0-3）：
            // 正常态直接告诉用户呼出键；注册失败态把「还能怎么呼出」写在
            // tooltip 里——鼠标悬停即见，不需要用户先猜
            let tooltip = if registered_ok {
                format!("zap — {hotkey_now} 呼出")
            } else {
                "热键被占用：左键点此呼出，右键进设置修改".to_string()
            };
            TrayIconBuilder::with_id("zap-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip(&tooltip)
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
                        let st = app.state::<AppState>();
                        // 显式回写勾选态：不押注 muda 在 MenuEvent 前自动翻转
                        // （各平台行为不一），与 save_settings 路径的 set_checked 同口径
                        if let Some(item) = st
                            .clip_item
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .as_ref()
                        {
                            let _ = item.set_checked(now);
                        }
                        // 托盘是 clip_enabled 的第三写者（评审 P1-18）：走 owner 单键形态
                        if let Err(e) = st.engine.set_clip_enabled(now) {
                            providers::diag::log_err(&format!("托盘切换剪贴板记录失败: {e}"));
                        }
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
            // 启动诊断行（评审 P1-10）：远程排障四问（版本/热键/注册成败/剪贴板
            // 开关）一条回答；watcher/索引等的后续异常都有了时间基准
            providers::diag::log(&format!(
                "startup\tversion={}\thotkey={hotkey_now}\tregistered={registered_ok}\tclip_enabled={clip_enabled}",
                app.package_info().version
            ));
            // 首启引导（评审 P0-3）：呼出热键的产品内露出只有设置页，而进设置
            // 的前提恰是先呼出——首装第一次自动呼出一次，第一眼即见「空面板
            // 即功能首页」。标记缺键即首启；与 single_instance 的二次启动呼出
            // 同路径，前端未就绪时窗口照常显示聚焦，列表由前端挂载自拉
            {
                let state = app.state::<AppState>();
                if state
                    .engine
                    .get_setting(providers::appconfig::KEY_FIRST_RUN_DONE)
                    .is_none()
                {
                    let _ = state
                        .engine
                        .set_setting(providers::appconfig::KEY_FIRST_RUN_DONE, "1");
                    providers::diag::log("startup\t首启引导：自动呼出面板");
                    toggle_panel(app.handle(), None);
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // 退出前 WAL checkpoint（评审 P1-08）：TRUNCATE 把 -wal 收缩回主库，
            // 库文件不会随长期运行无界增长；同步命令快速、失败静默（退出路径
            // 不因诊断动作失败而卡住）
            if matches!(event, tauri::RunEvent::Exit) {
                app.state::<AppState>().engine.wal_checkpoint();
            }
        });
}
