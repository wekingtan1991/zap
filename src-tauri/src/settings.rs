//! 应用设置域（自 lib.rs 拆出，评审 P1-17 步 1）：设置 DTO 与快照组装、
//! save_settings（热键热切换 + owner 应用 + 存档落库）、配置文件导入导出、
//! 透明面板特效。命令名与线协议不变——前端零改动。
//! 依赖 state.rs 的 AppState / DirectKind / WINDOW_LABEL。

use std::sync::atomic::Ordering;

use crate::providers::SearchEngine;
use crate::state::{AppState, DirectKind, WINDOW_LABEL};
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

// ---------- 应用设置 ----------

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSettingsDto {
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
    /// 拖入落区条松手后自动呼出中转站面板
    stagedrop_open_panel: bool,
    /// 落区条停靠角（bottom_right/bottom_left/top_right/top_left）
    stagedrop_position: String,
    pwd_clear_secs: i64,
    autostart: bool,
    everything_autostart: bool,
    /// 外观模式（dark/light/system）
    appearance: String,
    /// 透明面板（亚克力窗口特效）
    panel_transparency: bool,
    /// 可开关插件清单（仅工具插件；核心搜索源不列）
    plugins: Vec<crate::providers::PluginInfo>,
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
            .get_setting(crate::providers::appconfig::KEY_VAULT_HOTKEY)
            .unwrap_or_default(),
        clip_hotkey: engine
            .get_setting(crate::providers::appconfig::KEY_CLIP_HOTKEY)
            .unwrap_or_default(),
        superpanel_hotkey: engine
            .get_setting(crate::providers::appconfig::KEY_SUPERPANEL_HOTKEY)
            .unwrap_or_default(),
        stage_hotkey: engine
            .get_setting(crate::providers::appconfig::KEY_STAGE_HOTKEY)
            .unwrap_or_default(),
        clip_enabled: crate::providers::clipboard::CLIP_ENABLED.load(Ordering::Relaxed),
        clip_max: crate::providers::clipboard::RETENTION_LIMIT.load(Ordering::Relaxed),
        clip_autopaste: crate::providers::clipboard::PASTE_TO_PREV.load(Ordering::Relaxed),
        stage_max: crate::providers::stage::CAP.load(Ordering::Relaxed),
        stagedrop_enabled: crate::providers::stagedrop::STAGEDROP_ENABLED.load(Ordering::Relaxed),
        stagedrop_open_panel: crate::providers::stagedrop::STAGEDROP_OPEN_PANEL
            .load(Ordering::Relaxed),
        stagedrop_position: crate::providers::stagedrop::position_setting().to_string(),
        pwd_clear_secs: engine
            .get_setting(crate::providers::appconfig::KEY_PWD_CLEAR_SECS)
            .and_then(|v| v.parse().ok())
            .unwrap_or(crate::providers::appconfig::DEFAULT_PWD_CLEAR_SECS),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        everything_autostart: engine
            .get_setting(crate::providers::appconfig::KEY_EVERYTHING_AUTOSTART)
            .map(|v| v != "0")
            .unwrap_or(crate::providers::appconfig::DEFAULT_EVERYTHING_AUTOSTART),
        appearance: engine
            .get_setting(crate::providers::appconfig::KEY_APPEARANCE)
            .unwrap_or_else(|| "dark".into()),
        panel_transparency: engine
            .get_setting(crate::providers::appconfig::KEY_PANEL_TRANSPARENCY)
            .map(|v| v != "0")
            .unwrap_or(crate::providers::appconfig::DEFAULT_PANEL_TRANSPARENCY),
        plugins: engine.plugin_states(),
    }
}

/// 设置读取：注册表 + SQLite 读不占主线程（async 命令带 State 须返回
/// Result，Tauri 约定；前端调用方本就带 catch 兜底）
#[tauri::command]
pub(crate) async fn get_app_settings(
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
pub(crate) struct PluginSwitchDto {
    id: String,
    enabled: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveOutcome {
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
pub(crate) fn save_settings(
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
    // Option 容错：半新半旧的会话（前端未带上新键）保存时不再整单被拒
    stagedrop_open_panel: Option<bool>,
    // Option 容错：半新半旧的会话（前端未带上新键）保存时不再整单被拒
    stagedrop_position: Option<String>,
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
                    crate::providers::diag::log(&format!(
                        "hotkey\t呼出热键已切换 {current} → {hotkey}"
                    ));
                    state
                        .engine
                        .set_setting(crate::providers::appconfig::KEY_HOTKEY, &hotkey)?;
                    *state.hotkey.lock().expect("hotkey poisoned") = hotkey.clone();
                }
                Err(e) => {
                    crate::providers::diag::log_err(&format!("热键 {hotkey} 注册失败: {e}"));
                    hotkey_error = Some(format!("注册失败，可能已被其他程序占用：{e}"));
                    if let Ok(old) = current.parse::<Shortcut>() {
                        let _ = gs.register(old);
                    }
                }
            },
            Err(e) => {
                crate::providers::diag::log_err(&format!("热键 {hotkey} 无法解析: {e}"));
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
                crate::providers::appconfig::KEY_VAULT_HOTKEY,
                DirectKind::Vault,
            ),
            (
                "快速粘贴",
                clip_hotkey,
                crate::providers::appconfig::KEY_CLIP_HOTKEY,
                DirectKind::QuickPaste,
            ),
            (
                "超级面板",
                superpanel_hotkey,
                crate::providers::appconfig::KEY_SUPERPANEL_HOTKEY,
                DirectKind::SuperPanel,
            ),
            (
                "中转站直达",
                stage_hotkey,
                crate::providers::appconfig::KEY_STAGE_HOTKEY,
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

    // 剪贴板/中转站/落区条三组走 owner（评审 P1-18）——原子量与落库的镜像
    // 收敛在 SearchEngine 单处。None 参数（旧前端未带键）以当前生效值回填，
    // 语义与原「跳过不写」等价（同值重写无副作用）
    let clip_max = clip_max.clamp(10, 5000);
    state
        .engine
        .apply_clip_settings(&crate::providers::clipboard::ClipSettings {
            enabled: clip_enabled,
            max: clip_max,
            autopaste: clip_autopaste.unwrap_or_else(|| {
                crate::providers::clipboard::PASTE_TO_PREV.load(Ordering::Relaxed)
            }),
        })?;
    if let Some(cap) = stage_max {
        state.engine.apply_stage_cap(cap)?;
        // 调小立即收敛已有超容条目：等下次放入才收敛不符合「改了就生效」直觉
        // （调大无需动作，多出的空间天然可用）
        state.engine.stage_enforce_cap();
    }
    state
        .engine
        .apply_stagedrop_settings(&crate::providers::stagedrop::StagedropSettings {
            enabled: stagedrop_enabled
                .unwrap_or(crate::providers::stagedrop::STAGEDROP_ENABLED.load(Ordering::Relaxed)),
            open_panel: stagedrop_open_panel.unwrap_or(
                crate::providers::stagedrop::STAGEDROP_OPEN_PANEL.load(Ordering::Relaxed),
            ),
            position: stagedrop_position
                .clone()
                .unwrap_or_else(|| crate::providers::stagedrop::position_setting().to_string()),
        })?;
    if let Some(pos) = &stagedrop_position
        && crate::providers::stagedrop::Corner::parse(pos).is_some()
    {
        // 改了就生效：条子正显示时按新角重排（动画方向由下轮会话起点
        // 的 corner 推送自然跟上，无需在此发事件）
        crate::providers::stagedrop::relayout_if_shown(&app);
    }
    // 其余无原子量的存档键仍走单事务批量写（评审 P1-07 的 set_many）
    let mut kvs: Vec<(&'static str, String)> = Vec::new();
    if let Some(mode) = appearance {
        // 只收合法值（防前端脏数据把主题写坏）
        if matches!(mode.as_str(), "dark" | "light" | "system") {
            kvs.push((crate::providers::appconfig::KEY_APPEARANCE, mode));
        }
    }
    kvs.push((
        crate::providers::appconfig::KEY_PWD_CLEAR_SECS,
        pwd_clear_secs.to_string(),
    ));
    kvs.push((
        crate::providers::appconfig::KEY_EVERYTHING_AUTOSTART,
        if everything_autostart { "1" } else { "0" }.into(),
    ));
    state.engine.set_many(&kvs)?;
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

// ---------- 配置文件导出/导入 ----------

/// 设置快照 → 配置文件 settings 段（全量 Some：导出即当前生效值）
fn config_settings_from(dto: &AppSettingsDto) -> crate::providers::appconfig::ConfigSettings {
    crate::providers::appconfig::ConfigSettings {
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
        stagedrop_open_panel: Some(dto.stagedrop_open_panel),
        stagedrop_position: Some(dto.stagedrop_position.clone()),
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
pub(crate) async fn config_export(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    let hotkey = state.hotkey.lock().expect("hotkey poisoned").clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dto = build_app_settings(&app, &engine, &hotkey);
        let file = crate::providers::appconfig::ConfigFile {
            kind: crate::providers::appconfig::CONFIG_KIND.into(),
            version: crate::providers::appconfig::CONFIG_VERSION,
            exported_at: Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
            app_version: Some(app.package_info().version.to_string()),
            settings: config_settings_from(&dto),
            plugins: dto
                .plugins
                .iter()
                .map(|p| crate::providers::appconfig::ConfigPluginSwitch {
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
pub(crate) async fn config_import(
    path: String,
) -> Result<crate::providers::appconfig::ConfigFile, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("读取失败：{e}"))?;
        crate::providers::appconfig::validate_config_text(&text)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 透明面板（亚克力）：应用/清除窗口特效。系统不支持时返回 Err（调用方决定回退）。
/// 特效只作用于桌面模糊底，面板自身透明度由前端 CSS（zap-trans 类）负责
pub(crate) fn apply_window_effects(app: &AppHandle, on: bool) -> Result<(), String> {
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
pub(crate) async fn transparency_set(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    enable: bool,
) -> Result<(), String> {
    apply_window_effects(&app, enable)?;
    state.engine.set_setting(
        crate::providers::appconfig::KEY_PANEL_TRANSPARENCY,
        if enable { "1" } else { "0" },
    )
}
