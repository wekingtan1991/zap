//! 命令层（自 lib.rs 拆出，评审 P1-17 步 4）：约 50 个 Tauri 命令——搜索、
//! everything 托管、vault 全套、剪贴板两 kind、stage 全套、json/text 工具、
//! 超级面板、快捷方式、备份、secondary_action 与 open_item 分发。
//! 命令名与线协议不变——前端零改动。轻量开关外一律 async + spawn_blocking。

use base64::Engine as _;
use std::time::Duration;

use crate::hotkey_capture;
use crate::panel::{hide_panel_window, paste_bg, paste_text_async, take_prev_foreground};
use crate::providers::SearchItem;
use crate::state::{AppState, WINDOW_LABEL, spawn_hidden};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_opener::OpenerExt;

/// 命令层样板收编（审计 D6）：`spawn_blocking(move || body).await.map_err`
/// 三行结构此前逐命令手写 29 处。调用点保留 `let engine = state.engine.clone();`
/// ——macro_rules! 卫生规则下宏内绑定对 $body 不可见，engine 必须由调用点
/// 绑定（$body 在调用点解析、天然捕获）。宏内含 `.await`，仅 async fn 可用
macro_rules! blocking {
    ($body:expr) => {
        tauri::async_runtime::spawn_blocking(move || $body)
            .await
            .map_err(|e| e.to_string())
    };
}

#[tauri::command]
pub(crate) async fn search(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<SearchItem>, String> {
    // 搜索在阻塞线程池执行：SQLite、窗口枚举、Everything HTTP 都不占主线程
    let engine = state.engine.clone();
    blocking!(engine.search(&query))
}

/// 常驻工具 Dock 的入口清单：与搜索结果彻底分离，前端挂载/呼出/设置页关闭后拉取。
/// 只读插件原子开关与静态元信息，不碰 DB/HTTP；仍走阻塞池与同层命令保持一致
#[tauri::command]
pub(crate) async fn tool_entries(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<crate::providers::ToolEntry>, String> {
    let engine = state.engine.clone();
    blocking!(engine.tool_entries())
}

// 面板开关类命令（hide_panel/set_hold_panel/set_stage_view）已拆至 panel.rs
// （评审 P1-17 步 3），invoke_handler 引用 panel::* 路径；其余命令搬运口径
// 不变：涉及 DB/DPAPI/剪贴板/HTTP 的一律 async + spawn_blocking——Tauri 同步
// 命令跑在主线程，SQLite 与剪贴板重试窗口会卡住窗口消息泵（everything_status
// 的端口探测最坏可达数百毫秒）

#[tauri::command]
pub(crate) async fn everything_status() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(crate::providers::everything::status)
        .await
        .map_err(|e| e.to_string())
}

/// 自动检测并在需要时启动 Everything。按设置开关放行；已安装才启动，
/// 未安装不自动下载（留给面板里的手动入口）；失败 5 分钟内不重复尝试。
#[tauri::command]
pub(crate) async fn everything_ensure(state: tauri::State<'_, AppState>) -> Result<String, String> {
    // 设置读取一并压进阻塞线程（2026-11-04 评审 R4 P2）：get_setting 抢
    // conn 锁不做在 tokio worker 上
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let auto = engine
            .get_setting(crate::providers::appconfig::KEY_EVERYTHING_AUTOSTART)
            .map(|v| v != "0")
            .unwrap_or(crate::providers::appconfig::DEFAULT_EVERYTHING_AUTOSTART);
        if !auto {
            return Ok("disabled".into());
        }
        Ok(crate::providers::everything::ensure())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn vault_search(
    state: tauri::State<'_, AppState>,
    query: String,
) -> Result<Vec<SearchItem>, String> {
    let engine = state.engine.clone();
    blocking!(engine.vault_search(&query))
}

#[tauri::command]
pub(crate) async fn vault_add(
    state: tauri::State<'_, AppState>,
    title: String,
    username: String,
    password: String,
    url: String,
    notes: String,
    totp: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    blocking!({ engine.vault_add(&title, &username, &password, &url, &notes, &totp) })?
}

#[tauri::command]
pub(crate) async fn vault_action(
    state: tauri::State<'_, AppState>,
    id: i64,
    action: String,
) -> Result<String, String> {
    let engine = state.engine.clone();
    blocking!({
        let secs: i64 = engine
            .get_setting(crate::providers::appconfig::KEY_PWD_CLEAR_SECS)
            .and_then(|v| v.parse().ok())
            .unwrap_or(crate::providers::appconfig::DEFAULT_PWD_CLEAR_SECS);
        engine.vault_action(id, &action, secs)
    })?
}

/// 取单条账号完整记录（编辑预填；含解密后的密码明文与 TOTP 密钥）
#[tauri::command]
pub(crate) async fn vault_get(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<crate::providers::vault::VaultRecord, String> {
    let engine = state.engine.clone();
    blocking!(engine.vault_get(id))?
}

// Tauri 命令按字段平铺（前端 invoke 传同名参数），显式放行参数个数提示
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) async fn vault_update(
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
    blocking!({
        engine.vault_update(
            id, &title, &username, &password, &url, &notes, pinned, &totp,
        )
    })?
}

// ---------- 应用设置 ----------
// 设置域已拆至 settings.rs（评审 P1-17 步 1）：AppSettingsDto/快照组装/
// get_app_settings/save_settings/配置文件导入导出/透明面板特效。
// 命令名与线协议不变，invoke_handler 引用 crate::settings::*

#[tauri::command]
pub(crate) fn autostart_set(app: AppHandle, enable: bool) -> Result<(), String> {
    let al = app.autolaunch();
    if enable {
        al.enable().map_err(|e| e.to_string())
    } else {
        al.disable().map_err(|e| e.to_string())
    }
}

// ---------- 配置文件导出/导入 与 透明面板 ----------
// 同批拆至 settings.rs（config_settings_from/config_export/config_import/
// apply_window_effects/transparency_set），invoke_handler 引用 crate::settings::*

/// 超级面板动作执行（前端菜单点击；执行后由前端隐藏窗口）
#[tauri::command]
pub(crate) async fn superpanel_act(
    app: AppHandle,
    action: String,
    text: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::providers::superpanel::run_action(&app, &action, &text)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 超级面板当前菜单内容（前端聚焦窗口时拉取；事件推送之外的兜底通道）
#[tauri::command]
pub(crate) fn superpanel_payload() -> Option<crate::providers::superpanel::SuperActionsPayload> {
    crate::providers::superpanel::current_payload()
}

/// 收起超级面板（Esc / 点击项后调用）
#[tauri::command]
pub(crate) fn superpanel_hide(app: AppHandle) {
    crate::providers::superpanel::superpanel_hide(&app);
}

// ---------- 热键录制（低级键盘钩子，可捕获被输入法接管的组合如 Ctrl+Space） ----------

#[tauri::command]
pub(crate) fn start_hotkey_capture(app: AppHandle) {
    hotkey_capture::start(app);
}

#[tauri::command]
pub(crate) fn stop_hotkey_capture() {
    hotkey_capture::stop();
}

#[tauri::command]
pub(crate) async fn vault_parse(
    text: String,
) -> Result<Vec<crate::providers::vault::ParsedAccount>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::providers::vault::parse_blob(&text))
        .await
        .map_err(|e| e.to_string())
}

// ---------- JSON 工具插件 ----------

/// 一次返回分析报告 + 美化/压缩：前端每击键单次 IPC 单次解析
#[tauri::command]
pub(crate) async fn json_analyze_full(
    text: String,
) -> Result<crate::providers::jsontools::JsonFull, String> {
    tauri::async_runtime::spawn_blocking(move || crate::providers::jsontools::analyze_full(&text))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn json_to_yaml(text: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || crate::providers::jsontools::to_yaml(&text))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn json_to_ts(text: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || crate::providers::jsontools::to_ts(&text))
        .await
        .map_err(|e| e.to_string())?
}

// ---------- 文本工具域 ----------

/// 一次返回全部转换（时间戳/Base64/URL/哈希），前端按卡片渲染
#[tauri::command]
pub(crate) async fn tool_convert_all(
    text: String,
) -> Result<crate::providers::texttools::ToolBundle, String> {
    tauri::async_runtime::spawn_blocking(move || crate::providers::texttools::convert_all(&text))
        .await
        .map_err(|e| e.to_string())?
}

/// 生成一枚 UUID v4（按需点击生成，不随输入刷新）
#[tauri::command]
pub(crate) async fn tool_uuid() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(crate::providers::texttools::uuid_v4)
        .await
        .map_err(|e| e.to_string())?
}

// ---------- 端口工具域 ----------

/// 列出本地端口占用（默认只 LISTEN）。IP Helper 取表 + 进程查询都压后台线程
#[tauri::command]
pub(crate) async fn ports_list(
    filter: Option<u16>,
    only_listen: bool,
) -> Result<Vec<crate::providers::ports::PortEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::providers::ports::list_ports(filter, only_listen)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 结束占用端口的进程：后端重查归属 + 启动时间复核 + 保护名单，前端确认弹窗之外再挡一层
#[tauri::command]
pub(crate) async fn ports_kill(
    port: u16,
    pid: u32,
    started_at: Option<i64>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::providers::ports::kill_port(port, pid, started_at)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 识别条目回查原文：按行号读剪贴板历史（进入工作台时以识别时刻的内容为准）
#[tauri::command]
pub(crate) async fn clip_row(state: tauri::State<'_, AppState>, id: i64) -> Result<String, String> {
    let engine = state.engine.clone();
    blocking!({
        engine
            .clip_row(id)
            .ok_or_else(|| "该条剪贴板已被清理".to_string())
    })?
}

/// 删除单条剪贴板条目（kind 分流两表）：id 为完整条目 id（clip:123 /
/// clipimg:45），敏感内容误入库时的单条补救通道（评审 P1-09）
#[tauri::command]
pub(crate) async fn clip_delete(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    let engine = state.engine.clone();
    blocking!({
        let (kind, rowid) = id.split_once(':').ok_or("无效剪贴板条目")?;
        let rowid: i64 = rowid.parse().map_err(|_| "无效剪贴板条目")?;
        engine.clip_delete_row(kind, rowid)
    })?
}

/// 清空未固定剪贴板历史（文本+图片两表），返回删除条数：手动复制的敏感
/// 内容的批量补救通道（评审 P1-09），固定条目豁免
#[tauri::command]
pub(crate) async fn clip_clear_unpinned(
    state: tauri::State<'_, AppState>,
) -> Result<usize, String> {
    let engine = state.engine.clone();
    blocking!(engine.clip_clear_unpinned())?
}

/// 打开数据目录（评审 P1-16「关于」区）：版本/数据位置/诊断三要素的家，
/// 也是 SECURITY.md「设置→关于」引用的落地——用户排障取 zap.log 的入口
#[tauri::command]
pub(crate) async fn open_data_dir(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// 合并粘贴结果：count = 实际合并条数（已被清理的跳过）；pasted = 是否带了
/// 焦点交还+注入（自动粘贴关闭时只回填，措辞应说「到剪贴板」而非「粘贴」）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MergePasteOutcome {
    count: usize,
    pasted: bool,
}

/// 多条合并粘贴：按前端给定的点选顺序取各条全文，换行拼接后走与单条
/// 「选中即粘贴」同一套回填+注入流程（take_prev_foreground + paste_text_async）。
/// 合并产物是新组合，刻意不进自身复制抑制窗——与单条粘贴同口径，会收录进
/// 历史顶部。个别条目已被清理则跳过并返回实际条数（前端据此提示），
/// 全部已清理才报错
#[tauri::command]
pub(crate) async fn clip_merge_paste(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<MergePasteOutcome, String> {
    if ids.is_empty() {
        return Err("没有选择要合并的条目".into());
    }
    let engine = state.engine.clone();
    let texts = blocking!(engine.clip_texts(&ids))?;
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

/// 拖拽出结果：effect = OLE DROPEFFECT（1=COPY，0=取消/目标未收）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DragOutOutcome {
    effect: u32,
}

/// 拖出会话的守卫：hold_panel 与 DRAG_OUT_ACTIVE 成对释放（含 panic=abort 之外
/// 的所有早退路径；panic=abort 下无 unwind，残余 hold 由 panel 失焦 300ms 宽限等
/// 既有兜底收口——设计稿 §三 RAII 裁决）。持 AppHandle 才能跨 .await 保活到拖拽结束
struct DragOutGuard(AppHandle);

impl Drop for DragOutGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        self.0
            .state::<AppState>()
            .hold_panel
            .store(false, Ordering::Relaxed);
        crate::providers::dragout::DRAG_OUT_ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// 拖拽出：前端意图判定（mousedown + 位移阈值）后 invoke。命令体只做卫兵/备料/
/// hold 三件事；主线程闭包内先隐藏面板让开落点，拖拽结束按 effect 决定是否
/// 复现（effect=0 取消才复现）。拖拽循环经 run_on_main_thread 在**主线程**跑
///（WebView2 按下捕获所在线程——专用线程跨线程拖拽两轮真机证伪，见 dragout.rs
/// 模块头约束 5；DoDragDrop 内部自带泵，主线程 UI 不冻结）。effect 经 std mpsc
/// 回传，前端 promise settle 即解除 click 抑制器
#[tauri::command]
pub(crate) async fn drag_out_begin(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    kind: String,
    rowid: i64,
) -> Result<DragOutOutcome, String> {
    use std::sync::atomic::Ordering;
    crate::providers::diag::log(&format!("dragout: begin kind={kind} rowid={rowid}"));
    if crate::providers::dragout::DRAG_OUT_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err("已有拖出进行中".into());
    }
    // 守卫绑定名带下划线：拖拽在主线程同步跑（不再 spawn 线程 move guard），
    // 但 Drop 仍在命令作用域结束/早退时释放 hold 与会话标志
    let _guard = DragOutGuard(app.clone());
    // 卫兵：IPC 延迟内左键已松（快速甩动被前端抢判）→ 立即退出，前端照常走单击
    if !crate::providers::dragout::left_button_held() {
        crate::providers::diag::log("dragout: 卫兵拒绝（左键已松）");
        return Err("拖拽已取消".into());
    }
    // 备料两段式：DB 点查 owned 在 blocking 池；文本载荷零写盘（只回全文
    // String），图片才落临时 PNG（毫秒~百 ms 级）
    let engine = state.engine.clone();
    let payload = tauri::async_runtime::spawn_blocking(move || {
        crate::providers::dragout::prepare(&engine, &kind, rowid)
    })
    .await
    .map_err(|e| e.to_string())??;
    // 拖拽期间面板失焦即隐不触发（起拖即 hold，guard 释放）——同时也挡住
    // 下面主动 hide() 引发的 Focused(false)，避免二次收面板
    state.hold_panel.store(true, Ordering::Relaxed);
    // 拖拽必须在**主线程**跑（WebView2 按下捕获所在线程，模块头约束 5）——
    // 两轮真机证伪「专用线程」：跨线程 ole32 拖拽进模态循环却感知不到鼠标、
    // 阻塞不返回（用户「根本拖不动」）。DoDragDrop 内部自带泵，主线程 UI 不冻结
    //（Windows 拖拽的标准行为）。effect 经 std mpsc 回传（主线程同步 send），
    // async 端在 blocking 池纯等待——无 COM/STA 语义，不触发 STA-on-pool 反模式
    let (tx, rx) = std::sync::mpsc::channel::<u32>();
    // clone 一个 handle 作投递接收者：闭包内还要用 app 取窗口句柄，直接
    // `app.run_on_main_thread(move || …)` 会与接收者借用冲突（E0505）
    app.clone()
        .run_on_main_thread(move || {
            // 拖出期间把面板让开：面板浮层会挡住目标应用的落点。只 hide，不销毁
            // webview、不 emit shown、不重新定位——前端搜索词/视图/行状态原样保留。
            // hold_panel 已置位，hide 触发的 Focused(false) 不会再触发失焦即隐。
            // 早退路径（卫兵拒绝 / 备料失败）走不到这里，面板不会被误藏。
            let win = app.get_webview_window(WINDOW_LABEL);
            if let Some(ref w) = win {
                let _ = w.hide();
                crate::providers::diag::log("dragout: panel hidden");
            }

            // 文本载荷无临时文件；图片/中转站里只有本次生成的临时 PNG 需要延迟清理
            let cleanup = payload.temp_file();
            let effect = crate::providers::dragout::run_drag_out(&payload);
            if let Some(file) = cleanup {
                crate::providers::dragout::schedule_temp_cleanup(file);
            }

            // 取消才复现面板（原位置、原状态）；落地则保持隐藏等下次呼出
            if crate::providers::dragout::should_restore_panel(effect)
                && let Some(ref w) = win
            {
                let _ = w.show();
                let _ = w.set_focus();
                crate::providers::diag::log("dragout: cancelled, panel restored");
            }
            let _ = tx.send(effect);
        })
        .map_err(|e| e.to_string())?;
    let effect = tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "拖拽会话异常终止".to_string())?;
    Ok(DragOutOutcome { effect })
}

/// 通用文本复制（JSON 美化/压缩结果回填等）：
/// 走自身复制抑制窗，派生结果不重复进入剪贴板历史；写入带退避重试
#[tauri::command]
pub(crate) async fn copy_text(text: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::providers::clipboard::begin_self_copy();
        crate::providers::clipboard::retrying(|| {
            arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.clone()))
        })
        .map_err(|e| e.to_string())?;
        Ok("已复制".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 图片预览面板数据：预览图 PNG（base64，按 clipimage::PREVIEW_MAX 降采样）与
/// 原图元信息（width/height/size 描述原图，与另存为/仅复制/回贴的原图口径一致）。
/// 历史保留期内行号恒有效，已清理返回错误（面板内呈现，不收面板）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClipImagePreview {
    width: u32,
    height: u32,
    size: i64,
    created_at: i64,
    pinned: bool,
    png_base64: String,
}

/// 图片条目预览：预览面板进入时点查一次（与回贴同一行号寻址）。
/// BLOB 在 clip 锁内拖出，降采样/编码在锁外——16MB 原图 base64 过 IPC 约 22MB
/// 且前端 parse 峰值 3 份内存，降采样后大图预览收敛到 2MB 以内
#[tauri::command]
pub(crate) async fn clipimg_preview(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<ClipImagePreview, String> {
    let engine = state.engine.clone();
    blocking!({
        let m = engine
            .clip_image_meta(id)
            .ok_or_else(|| "该条剪贴板图片已被清理".to_string())?;
        let preview = crate::providers::clipimage::preview_png(&m.png)?;
        Ok(ClipImagePreview {
            width: m.width,
            height: m.height,
            size: m.size,
            created_at: m.created_at,
            pinned: m.pinned,
            png_base64: base64::engine::general_purpose::STANDARD.encode(preview),
        })
    })?
}

/// 图片仅复制不粘贴：历史原图放回剪贴板。走自身复制抑制窗——图已在历史，
/// 监听不应重复收录（与 copy_text 同口径）；解码与写入在后台线程
#[tauri::command]
pub(crate) async fn clipimg_copy(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    let engine = state.engine.clone();
    blocking!({
        let (w, h, png) = engine
            .clip_image(id)
            .ok_or_else(|| "该条剪贴板图片已被清理".to_string())?;
        let (_, _, rgba) = crate::providers::clipimage::decode_png(&png)?;
        crate::providers::clipboard::begin_self_copy();
        crate::providers::clipboard::set_image_dib(w as usize, h as usize, &rgba)?;
        Ok(())
    })?
}

/// 图片另存为 PNG：路径由前端 plugin-dialog save() 选定（与备份导出同模式），
/// 后端只负责把历史里的原始 PNG 落盘
#[tauri::command]
pub(crate) async fn clipimg_export(
    state: tauri::State<'_, AppState>,
    id: i64,
    path: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    blocking!({
        let (_, _, png) = engine
            .clip_image(id)
            .ok_or_else(|| "该条剪贴板图片已被清理".to_string())?;
        std::fs::write(&path, png).map_err(|e| format!("写入失败：{e}"))
    })?
}

/// 搜索捷径清单（设置页「搜索捷径」分区）：纯缓存读，无库访问
#[tauri::command]
pub(crate) fn shortcuts_list(
    state: tauri::State<AppState>,
) -> Vec<crate::providers::shortcuts::Shortcut> {
    state.engine.shortcuts_list()
}

/// 整单保存搜索捷径：校验 + 写库 + 热替换搜索缓存（写库走线程池）
#[tauri::command]
pub(crate) async fn shortcuts_save(
    state: tauri::State<'_, AppState>,
    list: Vec<crate::providers::shortcuts::Shortcut>,
) -> Result<(), String> {
    let engine = state.engine.clone();
    blocking!(engine.shortcuts_save(list))?
}

#[tauri::command]
pub(crate) async fn vault_import(
    state: tauri::State<'_, AppState>,
    entries: Vec<crate::providers::vault::ParsedAccount>,
) -> Result<usize, String> {
    let engine = state.engine.clone();
    blocking!(engine.vault_import(entries))?
}

// ---------- 账号库备份（加密导出/恢复） ----------

/// 加密导出全部账号到用户选定路径，返回条目数
#[tauri::command]
pub(crate) async fn vault_backup_export(
    state: tauri::State<'_, AppState>,
    path: String,
    passphrase: String,
) -> Result<usize, String> {
    let engine = state.engine.clone();
    blocking!(engine.vault_backup_export(&path, &passphrase))?
}

/// 账号库上次成功导出的 Unix 毫秒（0 = 从未备份）：设置页备份提醒的事实源
#[tauri::command]
pub(crate) async fn vault_last_export_at(state: tauri::State<'_, AppState>) -> Result<i64, String> {
    // spawn_blocking（2026-11-04 互验 V1 P3）：get_setting 抢 conn 锁不做在
    // tokio worker 上——备份导出长锁时会拖住整个命令系统
    let engine = state.engine.clone();
    blocking!(
        engine
            .get_setting(crate::providers::appconfig::KEY_VAULT_LAST_EXPORT)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    )
}

/// 解密备份文件为预览（条目 + 疑似重复标记，恢复前交前端确认）
#[tauri::command]
pub(crate) async fn vault_backup_decrypt(
    state: tauri::State<'_, AppState>,
    path: String,
    passphrase: String,
) -> Result<crate::providers::backup::BackupPreview, String> {
    let engine = state.engine.clone();
    blocking!(engine.vault_backup_decrypt(&path, &passphrase))?
}

/// 备份恢复：skip_duplicates 时跳过疑似重复，返回 (写入数, 跳过数)
#[tauri::command]
pub(crate) async fn vault_backup_restore(
    state: tauri::State<'_, AppState>,
    entries: Vec<crate::providers::backup::BackupEntry>,
    skip_duplicates: bool,
) -> Result<(usize, usize), String> {
    let engine = state.engine.clone();
    blocking!({ engine.vault_backup_restore(entries, skip_duplicates) })?
}

/// 「选中即粘贴」开关现值（vault 复制后是否收面板的依据，评审 P2-17）：
/// 读原子量零 IO，轻量到可随复制动作同调
#[tauri::command]
pub(crate) fn clip_autopaste_enabled() -> bool {
    crate::providers::clipboard::PASTE_TO_PREV.load(std::sync::atomic::Ordering::Relaxed)
}

/// 条目次级操作：admin=管理员运行 / reveal=所在文件夹定位 / copy_path=复制路径或链接 /
/// clip_copy=剪贴板条目仅复制（不粘贴） / close_window=关闭窗口。
/// .lnk（含 recent 的指针）先解析为真实目标；商店应用（shell:）不支持路径类操作
#[tauri::command]
pub(crate) async fn secondary_action(
    state: tauri::State<'_, AppState>,
    action: String,
    target: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    blocking!({
        // 非路径类操作先行：不参与 shell: 拦截与 lnk 解析
        match action.as_str() {
            // 历史条目按 rowid 回查全文回填（与选中即粘贴同口径：监听去重自动置顶），
            // 不交还原窗口、不注入 Ctrl+V
            "clip_copy" => {
                let id: i64 = target.parse().map_err(|_| "无效条目".to_string())?;
                let text = engine
                    .clip_row(id)
                    .ok_or_else(|| "该条剪贴板已被清理".to_string())?;
                crate::providers::clipboard::retrying(|| {
                    arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.clone()))
                })
                .map(|_| ())
            }
            "close_window" => {
                let hwnd: isize = target.parse().map_err(|_| "无效窗口".to_string())?;
                crate::providers::sysutil::close_window(hwnd)
            }
            _ => {
                if target.starts_with("shell:") {
                    return Err("商店应用不支持该操作".to_string());
                }
                let resolved = crate::providers::lnk::resolve_lnk_target(&target)
                    .unwrap_or_else(|| target.clone());
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
                        crate::providers::sysutil::run_as_admin(&exe)
                    }
                    "reveal" => crate::providers::sysutil::reveal_in_explorer(&resolved),
                    "copy_path" => crate::providers::clipboard::retrying(|| {
                        let resolved = resolved.clone();
                        arboard::Clipboard::new().and_then(|mut cb| cb.set_text(resolved))
                    })
                    .map(|_| ()),
                    _ => Err("未知操作".into()),
                }
            }
        }
    })?
}

#[tauri::command]
pub(crate) async fn everything_setup() -> Result<String, String> {
    // 整个流程（下载/解压/提权启动/等待）都在阻塞线程池

    tauri::async_runtime::spawn_blocking(crate::providers::everything::setup)
        .await
        .map_err(|e| e.to_string())?
}

// take_prev_foreground / paste_bg / paste_text_async / toggle_panel /
// open_stage_panel / quick_paste_latest / handle_stage_drop / foreground 系
// 已拆至 panel.rs（P1-17 步 3）

#[tauri::command]
pub(crate) fn open_item(app: AppHandle, item: SearchItem) -> Result<(), String> {
    // 剪贴板「选中即粘贴」在后台线程完成焦点交还与按键注入：open_item 在主线程，
    // 粘贴前的等待（焦点回落最长 ~600ms）不能压在这里
    let mut paste_started = false;
    let result = match item.kind.as_str() {
        "app" | "recent" | "file" | "folder" | "stage" => {
            // 启动计数（frecency 排序依据）：仅应用类，按 target 路径累计。
            // DB 写压 spawn_blocking（评审 P1-07）——frecency 无即时性要求，
            // 不占用同步命令的主线程窗口
            if item.kind == "app" {
                let engine = app.state::<AppState>().engine.clone();
                let target = item.target.clone();
                drop(tauri::async_runtime::spawn_blocking(move || {
                    engine.record_app_launch(&target);
                }));
            }
            if item.target.starts_with("shell:") {
                // UWP/商店应用：explorer 接管 shell:AppsFolder\<AUMID>
                let _ = std::process::Command::new("explorer")
                    .arg(&item.target)
                    .spawn();
                Ok(())
            } else if std::path::Path::new(&item.target).is_dir() {
                // 文件夹（含中转站里的目录条目）直开：不用 open crate 的目录
                // 分支——其 SHOpenFolderAndSelectItems 在本机实测静默 no-op 却
                // 返回 Ok（探针实证：同路径 start/explorer 正常），且语义是
                // 「父窗口里选中」而非「打开本目录」。explorer 直开 = 以目录
                // 为根的窗口，与上方 shell: 分支同构
                let _ = std::process::Command::new("explorer")
                    .arg(&item.target)
                    .spawn();
                Ok(())
            } else {
                app.opener().open_path(item.target.clone(), None::<&str>)
            }
        }
        "settings" => {
            // 白名单校验（评审 P2-08）：target 拼进 `cmd /C start`，必须出自
            // providers/settings.rs 静态清单——settings 条目的 target 恒由
            // 该表生成，表外值只可能来自前端脏数据
            if !crate::providers::settings::is_known_target(&item.target) {
                return Err("未知的系统设置项".into());
            }
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
            // 行号寻址（与列表线协议同构：target = 行号，与 clipimg 统一）。
            // 两段式（评审 P1-07）：主线程只做毫秒级 PK 存在性探查——「已被
            // 清理」要即时回前端 toast（不收面板）不能异步化，但全文回查
            // （MB 级锁内 String 拷贝）压到粘贴的后台线程，不占主线程消息泵
            let rowid = item.target.parse::<i64>().map_err(|_| {
                if item.kind == "clipimg" {
                    "无效图片条目".to_string()
                } else {
                    "无效剪贴板条目".to_string()
                }
            })?;
            let engine = app.state::<AppState>().engine.clone();
            let is_img = item.kind == "clipimg";
            let exists = if is_img {
                engine.clip_image_exists(rowid)
            } else {
                engine.clip_row_exists(rowid)
            };
            if !exists {
                return Err(if is_img {
                    "该条剪贴板图片已被清理".into()
                } else {
                    "该条剪贴板已被清理".into()
                });
            }
            // 选中即粘贴：焦点交还呼出前的窗口并注入 Ctrl+V，省掉手动一步
            let prev = take_prev_foreground(&app);
            paste_started = prev.is_some();
            if prev.is_some() {
                hide_panel_window(&app);
            }
            if is_img {
                // 图片的点查全图 + PNG 解码不能占主线程消息泵，压后台线程：
                // DIBV5 直写毫秒级，注入前的等待才不会随图放大成秒级
                let app_bg = app.clone();
                std::thread::spawn(move || {
                    // 取图/解码/DIB 写入任一失败即止（2026-11-04 P1）：照旧注入
                    // Ctrl+V 会把旧剪贴板内容贴进前台窗口；失败走全局 toast
                    let filled = if let Some((w, h, png)) = engine.clip_image(rowid)
                        && let Ok((_, _, rgba)) = crate::providers::clipimage::decode_png(&png)
                    {
                        crate::providers::clipboard::set_image_dib(w as usize, h as usize, &rgba)
                            .is_ok()
                    } else {
                        false
                    };
                    if filled {
                        if let Some(hwnd) = prev {
                            std::thread::sleep(Duration::from_millis(80));
                            crate::providers::winsys::paste_to_prev(hwnd);
                        }
                    } else {
                        let _ = app_bg.emit("zap://stage-drop-error", "图片回填失败，未执行粘贴");
                    }
                });
            } else {
                // 文本：后台线程回查全文再回填（与图片同构的两段式后半段）
                paste_bg(move || engine.clip_row(rowid), prev);
            }
            Ok(())
        }
        "win" => {
            if let Ok(hwnd) = item.target.parse::<isize>() {
                crate::providers::winsys::activate(hwnd);
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
pub(crate) async fn clip_pin(state: tauri::State<'_, AppState>, id: i64) -> Result<bool, String> {
    let engine = state.engine.clone();
    blocking!(engine.clip_pin(id))?
}

/// 图片条目固定/取消固定：直连图片表——clip_pin 的「文本表先命中」兜底链路
/// 在两表自增行号撞号时会固定到同号的文本行，图片条目必须按 kind 分流到此
#[tauri::command]
pub(crate) async fn clipimg_pin(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<bool, String> {
    let engine = state.engine.clone();
    blocking!(engine.clip_image_pin(id))?
}

// ---------- 中转站 ----------

#[tauri::command]
pub(crate) async fn stage_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<crate::providers::stage::StageRecord>, String> {
    let engine = state.engine.clone();
    blocking!(engine.stage_list())
}

/// 批量放入：逐条尝试，个别失败不影响其余（失败明细回传前端提示）
#[tauri::command]
pub(crate) async fn stage_add(
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
) -> Result<crate::providers::stage::StageAddOutcome, String> {
    let engine = state.engine.clone();
    blocking!(engine.stage_add(&paths))
}

/// 移出中转站（只删引用，不碰原文件）
#[tauri::command]
pub(crate) async fn stage_remove(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    let engine = state.engine.clone();
    blocking!(engine.stage_remove(id))?
}

/// 批量移出（清理失效条目）：失效判定在前端列表数据里（list 时已逐条 exists），
/// 这里只按 id 删除，返回实际移出条数
#[tauri::command]
pub(crate) async fn stage_remove_batch(
    state: tauri::State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<usize, String> {
    let engine = state.engine.clone();
    blocking!(engine.stage_remove_batch(&ids))
}

/// 清空临时区（固定条目保留），返回清掉条数
#[tauri::command]
pub(crate) async fn stage_clear(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    let engine = state.engine.clone();
    blocking!(engine.stage_clear())?
}

/// 固定/取消固定（返回操作后的状态）
#[tauri::command]
pub(crate) async fn stage_pin(state: tauri::State<'_, AppState>, id: i64) -> Result<bool, String> {
    let engine = state.engine.clone();
    blocking!(engine.stage_pin(id))?
}

/// 重新定位：为失效条目重绑路径
#[tauri::command]
pub(crate) async fn stage_relocate(
    state: tauri::State<'_, AppState>,
    id: i64,
    path: String,
) -> Result<(), String> {
    let engine = state.engine.clone();
    blocking!(engine.stage_relocate(id, &path))?
}

/// 复制文件到剪贴板（CF_HDROP，可粘贴到任意位置）；无库访问但剪贴板写入带重试，照例压线程池
#[tauri::command]
pub(crate) async fn stage_copy_file(paths: Vec<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::providers::stage::copy_files_to_clipboard(&paths)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 落区条当前反馈文案（前端挂载时拉取同步；None = 默认文案）。
/// drop 的接收与 stage_add 全在 Rust 侧，这是唯一的前端拉取点
#[tauri::command]
pub(crate) fn stagedrop_feedback_state() -> Option<String> {
    crate::providers::stagedrop::current_text()
}

/// 批量撤销恢复：原样插回刚移出/清空的条目（含固定态与时间戳；路径已被
/// 重新放入的幂等跳过），返回恢复条数。载荷 RestoreItem 在 stage.rs
/// （审计 B3 收编，camelCase 线协议不变）
#[tauri::command]
pub(crate) async fn stage_restore_batch(
    state: tauri::State<'_, AppState>,
    items: Vec<crate::providers::stage::RestoreItem>,
) -> Result<usize, String> {
    let engine = state.engine.clone();
    blocking!(engine.stage_restore_batch(items))?
}
