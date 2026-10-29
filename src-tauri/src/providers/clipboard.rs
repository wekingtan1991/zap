use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use clipboard_master::{CallbackResult, ClipboardHandler, Master};
use rusqlite::{Connection, params};
use windows::Win32::System::DataExchange::GetClipboardOwner;

use super::SearchItem;

/// 账号库复制（密码/用户名/备注）时开启：窗口期内的剪贴板变更不入库（阅后即焚）。
/// 用时间窗而非单次标记：密码复制会连续写多次剪贴板（明文 + 2 个排除格式），
/// 每次都触发变更事件，单次标记只能挡第一条。
pub static SUPPRESS_CLIP_UNTIL: AtomicU64 = AtomicU64::new(0);

/// 当前是否处于「zap 自身复制」抑制窗口
pub fn in_suppress_window() -> bool {
    let until = SUPPRESS_CLIP_UNTIL.load(Ordering::Relaxed);
    until != 0 && (now_ms() as u64) < until
}

/// 标记一次 zap 自身复制：后续 1.5s 内的剪贴板变更事件不入库
pub fn begin_self_copy() {
    SUPPRESS_CLIP_UNTIL.store(now_ms() as u64 + 1500, Ordering::Relaxed);
}

/// 剪贴板写入重试（open + set 整操作退避）。OpenClipboard 是全局独占资源，
/// IM/浏览器常驻监听或运行中的另一 zap 可能瞬时占用；arboard 内部仅 5×5ms
/// 重试窗口太窄——这里 5 次递进退避（合计约 370ms）。与 vault.rs 焚除的
/// 6 次重试同源教训；写路径是同步用户操作，退避刻意短，不卡手感。
/// op 内部自行 Clipboard::new()——每次重试全新打开，比持有句柄更稳
pub fn retrying<T>(mut op: impl FnMut() -> Result<T, arboard::Error>) -> Result<T, String> {
    const BACKOFF_MS: [u64; 5] = [0, 20, 50, 100, 200];
    let mut last = String::new();
    for ms in BACKOFF_MS {
        if ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

/// 敏感内容专用复制：排除 Windows 剪贴板历史（Win+V）与云剪贴板同步。
/// 不排 monitor：zap 自身的剪贴板监听依赖变更事件做入库抑制（begin_self_copy
/// 时间窗），排 monitor 有让监听收不到事件、抑制错位的风险。
/// vault 密码/用户名/备注与超级面板复制统一走此通道（评审 P1-13）——
/// 凡「内容来自受保护域」的复制都不进 Win+V 与云剪贴板
#[cfg(windows)]
pub fn copy_text_private(text: &str) -> Result<(), String> {
    use arboard::SetExtWindows;
    let owned = text.to_string();
    retrying(|| {
        arboard::Clipboard::new().and_then(|mut cb| {
            cb.set()
                .exclude_from_history()
                .exclude_from_cloud()
                .text(owned.clone())
        })
    })
    .map(|_| ())
}

#[cfg(not(windows))]
pub fn copy_text_private(text: &str) -> Result<(), String> {
    retrying(|| {
        let owned = text.to_string();
        arboard::Clipboard::new().and_then(|mut cb| cb.set_text(owned))
    })
    .map(|_| ())
}

/// 剪贴板图片直写 CF_DIBV5，绕开 arboard set_image——它在 DIBV5 之外还用
/// image 库编码一份 PNG 格式，39MB 长截图仅编码就要 5-6 秒（「仅复制没反应」
/// 的实测根因：按钮全程静默，粘贴路径的 Ctrl+V 注入也跟着迟到）。DIBV5 直写
/// 毫秒级；BI_BITFIELDS 掩码保透明通道，正高度+行翻转规避 Word 负高度坑
/// （arboard 同款注释）。不带 PNG 格式：个别「只认 PNG」的目标会退化读
/// DIBV5，换取常用图即时可贴。错误为 String 型故不与 retrying（arboard::Error）
/// 共用，退避节奏与之一致
pub fn set_image_dib(width: usize, height: usize, rgba: &[u8]) -> Result<(), String> {
    const BACKOFF_MS: [u64; 5] = [0, 20, 50, 100, 200];
    if width == 0 || height == 0 || rgba.len() != width * height * 4 {
        return Err("图片尺寸与数据长度不符".into());
    }
    let payload = build_dibv5(width, height, rgba);
    let mut last = String::new();
    for ms in BACKOFF_MS {
        if ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
        match write_dibv5(&payload) {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// 构造 CF_DIBV5 载荷：BITMAPV5HEADER(124B) + 像素。RGBA→BGRA + 行序翻转为
/// DIB 要求的 bottom-up。纯函数，测试锁定字节布局
fn build_dibv5(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    use windows::Win32::Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER};
    debug_assert_eq!(rgba.len(), width * height * 4);
    // RGBA → BGRA（原地交换 R/B 通道）
    let mut px = rgba.to_vec();
    for p in px.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    // 行序翻转 bottom-up（原地首尾行交换）
    let row = width * 4;
    let mut tmp = vec![0u8; row];
    for y in 0..height / 2 {
        let a = y * row;
        let b = (height - 1 - y) * row;
        tmp.copy_from_slice(&px[a..a + row]);
        px.copy_within(b..b + row, a);
        px[b..b + row].copy_from_slice(&tmp);
    }
    // arboard：LCS_sRGB 魔数在 windows-rs 缺失（microsoft/windows-rs#2711）
    const LCS_SRGB: u32 = 0x7352_4742;
    const LCS_GM_IMAGES: u32 = 0x0000_0004;
    let header = BITMAPV5HEADER {
        bV5Size: std::mem::size_of::<BITMAPV5HEADER>() as u32,
        bV5Width: width as i32,
        bV5Height: height as i32,
        bV5Planes: 1,
        bV5BitCount: 32,
        bV5Compression: BI_BITFIELDS,
        bV5SizeImage: (4 * width * height) as u32,
        bV5XPelsPerMeter: 0,
        bV5YPelsPerMeter: 0,
        bV5ClrUsed: 0,
        bV5ClrImportant: 0,
        bV5RedMask: 0x00ff_0000,
        bV5GreenMask: 0x0000_ff00,
        bV5BlueMask: 0x0000_00ff,
        bV5AlphaMask: 0xff00_0000,
        bV5CSType: LCS_SRGB,
        bV5Endpoints: Default::default(),
        bV5GammaRed: 0,
        bV5GammaGreen: 0,
        bV5GammaBlue: 0,
        bV5Intent: LCS_GM_IMAGES,
        bV5ProfileData: 0,
        bV5ProfileSize: 0,
        bV5Reserved: 0,
    };
    let header_bytes = unsafe {
        std::slice::from_raw_parts(
            (&raw const header).cast::<u8>(),
            std::mem::size_of::<BITMAPV5HEADER>(),
        )
    };
    let mut out = Vec::with_capacity(header_bytes.len() + px.len());
    out.extend_from_slice(header_bytes);
    out.extend_from_slice(&px);
    out
}

/// 单次写入：Open → Empty → GlobalAlloc+copy → SetClipboardData(CF_DIBV5)。
/// SetClipboardData 成功后 hmem 归剪贴板所有，失败须 GlobalFree 兜底
fn write_dibv5(payload: &[u8]) -> Result<(), String> {
    use windows::Win32::Foundation::GlobalFree;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    // CF_DIBV5 在 Ole（windows-rs 分组与 MSDN 页不同源）
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::CF_DIBV5;
    unsafe {
        OpenClipboard(None).map_err(|e| format!("打开剪贴板失败：{e}"))?;
        if let Err(e) = EmptyClipboard() {
            let _ = CloseClipboard();
            return Err(format!("清空剪贴板失败：{e}"));
        }
        let h = match GlobalAlloc(GMEM_MOVEABLE, payload.len()) {
            Ok(h) => h,
            Err(e) => {
                let _ = CloseClipboard();
                return Err(format!("分配剪贴板内存失败：{e}"));
            }
        };
        let p = GlobalLock(h);
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            let _ = CloseClipboard();
            return Err("GlobalLock 失败".into());
        }
        std::ptr::copy_nonoverlapping(payload.as_ptr(), p.cast(), payload.len());
        let _ = GlobalUnlock(h);
        match SetClipboardData(CF_DIBV5.0 as u32, Some(HANDLE(h.0))) {
            Ok(_) => {
                let _ = CloseClipboard();
                Ok(())
            }
            Err(e) => {
                let _ = GlobalFree(Some(h));
                let _ = CloseClipboard();
                Err(format!("写入剪贴板失败：{e}"))
            }
        }
    }
}

/// 剪贴板历史总开关（设置页/托盘切换），关闭时监听仍运行但不入库
pub static CLIP_ENABLED: AtomicBool = AtomicBool::new(true);

/// 「选中即粘贴」开关：剪贴板条目选中后焦点交还原窗口并发送 Ctrl+V。
/// 关闭时退化为仅回填剪贴板
pub static PASTE_TO_PREV: AtomicBool = AtomicBool::new(true);

/// 历史保留条数（设置可调）
pub static RETENTION_LIMIT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(500);

/// 已知密码管理器的进程名标记：来源命中的复制不落盘（第一道隐私防线）
const PASSWORD_MARKERS: [&str; 8] = [
    "keepass",
    "bitwarden",
    "1password",
    "enpass",
    "dashlane",
    "lastpass",
    "roboform",
    "keyguard",
];

pub fn open_db(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        // busy_timeout 显式化（评审 P1-07）：rusqlite 默认已是 5000ms，但那是
        // 隐式依赖——显式设 1500ms 把「写-写冲突时最多等 1.5s」变成明文契约，
        // 宁可主线程命令快败重试，不让同步命令被 16MB 大图事务拖满 5 秒。
        // 放在 journal_mode 之前，让 WAL 设置本身也受保护
        "PRAGMA busy_timeout=1500;
         PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS clipboard_history (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             content TEXT NOT NULL UNIQUE,
             created_at INTEGER NOT NULL
         );",
    )?;
    super::clipimage::init_table(&conn);
    if let Err(e) = migrate(&conn) {
        super::diag::log_err(&format!("db\tschema 迁移失败: {e}"));
    }
    Ok(conn)
}

/// 当前 schema 版本：每次破坏性/加列演进 +1，对应 migrate 里的 `v < N` 步骤
const SCHEMA_VERSION: i64 = 1;

/// schema 迁移器（评审 P1-14）：PRAGMA user_version 单向递增，BEGIN IMMEDIATE
/// 内重读版本 → 按 `v < N` 逐步执行 → 写回 → commit。三个连接（engine/监听/
/// 图片 worker）并发调用天然幂等——后到者在事务内看到已升级的版本号直接
/// 跳过。旧版「吞错 ALTER」收编为步骤 1：新库建表已走同款 ALTER（列已存在
/// 时该语句失败但事务内语句失败只回滚自身，版本照常写回）
fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if v < 1 {
            // 步骤 1（v0→v1）：历史行固定标记列。已存在时失败可忽略（同旧版语义）
            let _ = conn.execute_batch(
                "ALTER TABLE clipboard_history ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
            );
        }
        conn.execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION}"))
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(|e| e.to_string()),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// 带损坏自愈的打开（评审 P0-1②）：open 成功还要过 quick_check——页头有效
/// 但页损坏时建表可能侥幸通过，首查才炸，把「能打开」当「健康」会留下更深的
/// 静默故障。判定失败则关闭连接后把库连同 -wal/-shm 三件套隔离改名
/// （zap.db.broken-<ts>[…]），现场保留供诊断，重建空库继续启动。
/// SearchEngine::new 与两个监听线程都走这里：主路径自愈后，后启动的
/// watcher 打开的已是新库。重建再失败只能是磁盘/权限级故障，向上传错误。
pub fn open_db_resilient(path: &Path) -> Result<Connection, String> {
    match try_open_checked(path) {
        Ok(conn) => Ok(conn),
        Err(why) => {
            super::diag::log(&format!(
                "db\t库不健康，隔离重建\t{why}\t{}",
                path.display()
            ));
            quarantine_db(path);
            open_db(path).map_err(|e| {
                super::diag::log(&format!("db\t隔离后重建仍失败\t{e}\t{}", path.display()));
                format!("隔离重建仍失败: {e}（原始问题: {why}）")
            })
        }
    }
}

fn try_open_checked(path: &Path) -> Result<Connection, String> {
    let conn = open_db(path).map_err(|e| e.to_string())?;
    // quick_check 轻量校验（B-tree 结构级，不读全量数据）；非 "ok" 一律按坏库处理
    match conn.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)) {
        Ok(s) if s == "ok" => Ok(conn),
        Ok(s) => Err(format!("quick_check: {s}")),
        Err(e) => Err(format!("quick_check 执行失败: {e}")),
    }
}

/// 坏库三件套改名隔离。Windows 下文件被占用时改名失败，尽力而为逐个处理：
/// 主文件改不动时 -wal/-shm 大概率同样改不动，重建 open 会原样报错给上层。
fn quarantine_db(path: &Path) {
    let ts = now_ms();
    for suffix in ["", "-wal", "-shm"] {
        let mut src = std::ffi::OsString::from(path.as_os_str());
        src.push(suffix);
        let src = PathBuf::from(src);
        if !src.exists() {
            continue;
        }
        let mut dst = std::ffi::OsString::from(path.as_os_str());
        dst.push(format!(".broken-{ts}{suffix}"));
        match std::fs::rename(&src, PathBuf::from(dst)) {
            Ok(()) => super::diag::log(&format!("db\t已隔离\t{}\t{}", src.display(), suffix)),
            Err(e) => {
                super::diag::log(&format!("db\t隔离失败\t{}\t{e}", src.display()));
                // 主文件改不动就别留半套现场：伴生文件留在原地与新库同名会
                // 被 SQLite 当成恢复源——能删则删
                if !suffix.is_empty() {
                    let _ = std::fs::remove_file(&src);
                }
            }
        }
    }
}

/// 待入库的图片任务：回调线程只负责取 RGBA，编码/哈希/缩略图/写库在独立线程
struct ImageTask {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/// 图片入库 worker：channel 断开（监听线程退出）后归还连接，测试可内联驱动
fn image_store_loop(conn: Connection, rx: std::sync::mpsc::Receiver<ImageTask>) -> Connection {
    for task in rx {
        let _ = super::clipimage::store(&conn, &task.rgba, task.width, task.height);
    }
    conn
}

/// 启动剪贴板监听线程：独立消息循环 + 独立写连接（WAL 下与读连接并存）。
/// 图片入库另起一线程：PNG 编码 + sha256 + 缩略图对 4K 截图可达数百毫秒，
/// 同步跑在回调里会拖住监听，截完图立刻呼出面板可能赶不上入库——回调只取
/// RGBA 丢队列。队列有界（4 张 4K RGBA ≈ 130MB 上限）防截图风暴打爆内存，
/// 满即丢新图：风暴下入库本就追不上，静默优于阻塞监听
pub fn spawn_watcher(db_path: PathBuf) {
    let (img_tx, img_rx) = std::sync::mpsc::sync_channel::<ImageTask>(4);
    let worker_path = db_path.clone();
    std::thread::spawn(move || {
        let conn = match open_db_resilient(&worker_path) {
            Ok(c) => c,
            Err(e) => {
                super::diag::log_err(&format!("剪贴板图片入库线程打开数据库失败: {e}"));
                return;
            }
        };
        let _ = image_store_loop(conn, img_rx);
    });
    std::thread::spawn(move || {
        // watcher 自愈（评审 P1-11）：master.run() 退出（消息循环异常）或
        // arboard 初始化失败都曾是「线程静默死亡」——剪贴板历史无声停摆，
        // 唯一线索是 release 下没人看得见的 eprintln。respawn 循环 + 上限
        // 3 次退避（5s 间隔）：持续失败说明系统性故障（如剪贴板被别的
        // 进程锁死），无限重试只会空转刷日志
        const MAX_RESPAWN: u32 = 3;
        for attempt in 0..=MAX_RESPAWN {
            if attempt > 0 {
                super::diag::log_warn(&format!(
                    "watcher 第 {attempt}/{MAX_RESPAWN} 次 respawn（5s 后）"
                ));
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
            let conn = match open_db_resilient(&db_path) {
                Ok(c) => c,
                Err(e) => {
                    super::diag::log_err(&format!("剪贴板数据库打开失败: {e}"));
                    continue;
                }
            };
            struct Handler {
                conn: Connection,
                cb: arboard::Clipboard,
                img_tx: std::sync::mpsc::SyncSender<ImageTask>,
            }
            impl ClipboardHandler for Handler {
                fn on_clipboard_change(&mut self) -> CallbackResult {
                    // 账号库阅后即焚：自身复制窗口内的变更一律不入库（高频路径，不落日志）
                    if in_suppress_window() {
                        return CallbackResult::Next;
                    }
                    // 总开关关闭：照常消费事件但不入库
                    if !CLIP_ENABLED.load(Ordering::Relaxed) {
                        return CallbackResult::Next;
                    }
                    if let Ok(text) = self.cb.get_text() {
                        let text = text.trim_end().to_string();
                        // 账号库密码防护（评审 P1-09）：来源过滤只认桌面密码管理器，
                        // 浏览器内置 PM/用户手动复制的密码会漏——入库前与 vault
                        // 密码明文集比对，命中不入库并顺手清掉历史同值行（改密/
                        // 导入后的旧值同样不可从历史翻出）。比对走 O(1) 内存集，
                        // 首次惰性全量解密一次
                        let vault_hit =
                            !text.is_empty() && super::vault::password_cache_hit(&self.conn, &text);
                        if vault_hit {
                            let _ = self.conn.execute(
                                "DELETE FROM clipboard_history WHERE content=?1",
                                params![text],
                            );
                            return CallbackResult::Next;
                        }
                        if !text.is_empty() && !is_from_password_manager() {
                            let now = now_ms();
                            let _ = self.conn.execute(
                                "INSERT INTO clipboard_history(content, created_at) VALUES(?1, ?2)
                                 ON CONFLICT(content) DO UPDATE SET created_at=excluded.created_at",
                                params![text, now],
                            );
                            // 清理让位于固定：置顶条目不参与保留条数淘汰
                            let _ = self.conn.execute(
                                "DELETE FROM clipboard_history WHERE pinned=0 AND id NOT IN (
                                     SELECT id FROM clipboard_history ORDER BY created_at DESC LIMIT ?1
                                 )",
                                params![RETENTION_LIMIT.load(Ordering::Relaxed)],
                            );
                        }
                    } else if let Ok(img) = self.cb.get_image() {
                        // 无文本可取时尝试图片（截图类）：只取 RGBA 丢队列，编码/哈希/
                        // 缩略图在独立线程；队列满丢弃（风暴保护，见 spawn_watcher 注释）
                        let _ = self.img_tx.try_send(ImageTask {
                            rgba: img.bytes.into_owned(),
                            width: img.width as u32,
                            height: img.height as u32,
                        });
                    }
                    CallbackResult::Next
                }

                fn on_clipboard_error(&mut self, error: io::Error) -> CallbackResult {
                    super::diag::log_warn(&format!("剪贴板监听错误: {error}"));
                    CallbackResult::Next
                }
            }

            let cb = match arboard::Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    super::diag::log_err(&format!("剪贴板初始化失败: {e}"));
                    continue; // respawn 再试
                }
            };
            let handler = Handler {
                conn,
                cb,
                img_tx: img_tx.clone(),
            };
            match Master::new(handler) {
                Ok(mut master) => {
                    super::diag::log("watcher\t监听启动");
                    if let Err(e) = master.run() {
                        super::diag::log_err(&format!("剪贴板监听退出: {e}"));
                    }
                }
                Err(e) => super::diag::log_err(&format!("剪贴板监听启动失败: {e}")),
            }
        }
        super::diag::log_err("watcher 放弃 respawn：连续退出已达上限，剪贴板历史停摆");
    });
}

/// 检查本次复制是否来自密码管理器（读剪贴板归属窗口的进程路径）
fn is_from_password_manager() -> bool {
    unsafe {
        let Ok(owner) = GetClipboardOwner() else {
            return false;
        };
        if owner.is_invalid() {
            return false;
        }
        let Some(path) =
            super::sysutil::hwnd_pid(owner).and_then(super::sysutil::process_path_of_pid)
        else {
            return false;
        };
        let path = path.to_lowercase();
        PASSWORD_MARKERS.iter().any(|m| path.contains(m))
    }
}

/// 搜索历史：Rust 侧统一打分（含拼音）。空查询返回最近条目（固定置顶，呼出即见常用）；
/// 文本查询下图片条目按 标题+副标题（含尺寸）参与打分同池竞争。
/// 列表只吃前 256 字符（SQL 侧 substr）与换行数（SQL 侧现算）——全文不随列表过
/// IPC（几 MB 的粘贴曾每次呼出/每击键整行拖回），回贴/仅复制按行号回查
/// （open_item 与 secondary_action 的 clip 分支）
pub fn search(conn: &Connection, query: &str, limit: usize) -> Vec<SearchItem> {
    let now = now_ms();
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, created_at, pinned, substr(content, 1, 256),
                length(content) - length(replace(content, char(10), ''))
         FROM clipboard_history
         ORDER BY pinned DESC, created_at DESC LIMIT 300",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
        ))
    }) else {
        return Vec::new();
    };
    let rows: Vec<(i64, i64, i64, String, i64)> = rows.flatten().collect();

    if query.is_empty() {
        // 图片与文本条目按 固定 > 新旧 合并（两表各自已按该序取回）；
        // 图片上限取一半，避免连串截图挤掉文本常用项
        let mut merged: Vec<(i64, i64, SearchItem)> = rows
            .iter()
            .take(limit)
            .map(|(id, t, pinned, head, nl)| {
                (
                    *pinned,
                    *t,
                    build_item(*id, head, *nl, *t, now, 0, *pinned == 1),
                )
            })
            .collect();
        merged.extend(super::clipimage::search_recent(conn, limit - limit / 2));
        merged.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        return merged
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(i, (_, _, mut it))| {
                it.score = 60.0 - i as f32;
                it
            })
            .collect();
    }

    let q = query.trim().to_lowercase();
    // 打分池：文本行按 head 打分；图片条目按 标题+副标题 打分同池竞争——
    // 标题含尺寸（「图片 1920×1080」），按尺寸/「图片」找图可以召回，
    // 图片不再是空查询专属（先构造后排序：池内元素统一为现成条目）
    let mut scored: Vec<(f32, i64, i64, SearchItem)> = rows
        .iter()
        .filter_map(|(id, t, pinned, head, nl)| {
            super::score_lowered(head, &q).map(|s| {
                (
                    s,
                    *pinned,
                    *t,
                    build_item(*id, head, *nl, *t, now, 0, *pinned == 1),
                )
            })
        })
        .collect();
    for (pinned, t, it) in super::clipimage::search_recent(conn, limit) {
        let hay = format!("{} {}", it.title, it.subtitle);
        if let Some(s) = super::score_lowered(&hay, &q) {
            scored.push((s, pinned, t, it));
        }
    }
    // 同分档内固定优先，其次新的在前（与账号库口径一致）
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap()
            .then(b.1.cmp(&a.1))
            .then(b.2.cmp(&a.2))
    });
    scored.truncate(limit);
    scored
        .into_iter()
        .enumerate()
        .map(|(i, (_, _, _, mut it))| {
            it.score = 60.0 - i as f32;
            it
        })
        .collect()
}

/// 条目构造：只吃首行（head 前 256 字符足够——标题上限 60）与换行数，全文不进
/// 条目。target = 行号：与 clipimg 同构的行号寻址，内容本体只在动作时按行号
/// 点查（回贴 open_item / 仅复制 secondary_action）——列表与 IPC 不再携带全文
fn build_item(
    rowid: i64,
    head: &str,
    newline_count: i64,
    created_at: i64,
    now: i64,
    index: usize,
    pinned: bool,
) -> SearchItem {
    let first_line = head.lines().next().unwrap_or("").trim().to_string();
    let title = if first_line.chars().count() > 60 {
        format!("{}…", first_line.chars().take(60).collect::<String>())
    } else if first_line.is_empty() {
        "(空白)".into()
    } else {
        first_line
    };
    let extra_lines = newline_count.max(0) as usize;
    let subtitle = if extra_lines > 0 {
        format!(
            "剪贴板 · {} · 还有 {} 行",
            ago(created_at, now),
            extra_lines
        )
    } else {
        format!("剪贴板 · {}", ago(created_at, now))
    };
    SearchItem {
        // rowid 寻址：固定/回查等跨列表操作不怕列表重排（旧位置 id 重扫即漂移）
        id: format!("clip:{rowid}"),
        title,
        subtitle,
        kind: "clip".into(),
        target: rowid.to_string(),
        icon: None,
        pinned,
        score: 60.0 - index as f32,
    }
}

/// 固定/取消固定（返回操作后的状态）：固定条目列表置顶，且不参与保留条数淘汰。
/// 图片条目不走这里——文本表先命中的兜底链路在两表自增行号撞号时会固定到
/// 同号的文本行；前端按 kind 分流到 clip_image_pin 直连，此兜底仅存防御
pub fn pin_toggle(conn: &Connection, id: i64) -> Result<bool, String> {
    let changed = conn
        .execute(
            "UPDATE clipboard_history SET pinned = 1 - pinned WHERE id=?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
    if changed > 0 {
        let pinned: i64 = conn
            .query_row(
                "SELECT pinned FROM clipboard_history WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        return Ok(pinned == 1);
    }
    super::clipimage::pin_toggle(conn, id)
}

/// 相对时间文案（图片条目副标题复用）
pub(crate) fn ago(then: i64, now: i64) -> String {
    let secs = (now - then).max(0) / 1000;
    if secs < 60 {
        "刚刚".into()
    } else if secs < 3600 {
        format!("{} 分钟前", secs / 60)
    } else if secs < 86400 {
        format!("{} 小时前", secs / 3600)
    } else {
        format!("{} 天前", secs / 86400)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 最新一条历史的 (行号, 内容)：面板呼出时内容识别与快速粘贴的输入。
/// 按时间取而非按 id：入库是 content 唯一约束的 upsert（重抄旧内容只更新
/// created_at 不产生新 id），按 id 排序会返回上一条新插入的而非刚复制的。
/// 无历史（首次使用/已清空）返回 None
pub fn latest(conn: &Connection) -> Option<(i64, String)> {
    conn.query_row(
        "SELECT id, content FROM clipboard_history ORDER BY created_at DESC, id DESC LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
}

/// 按行号取原文：识别条目进入工作台后回查当时的内容（历史保留期内行号恒有效）
pub fn get_by_id(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row(
        "SELECT content FROM clipboard_history WHERE id=?1",
        params![id],
        |r| r.get(0),
    )
    .ok()
}

/// 剪贴板历史源插件适配：与 apps 同为 needs_conn 的常规源，
/// 引擎在 clip 锁短暂持有内调用（与 HTTP/窗口枚举隔离）
pub struct ClipboardPlugin;

impl super::plugin::Plugin for ClipboardPlugin {
    fn needs_conn(&self) -> bool {
        true
    }
    fn search(&self, query: &str, conn: Option<&Connection>) -> Vec<SearchItem> {
        match conn {
            Some(c) => search(c, query, 6),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn mem_db() -> Connection {
        open_db(Path::new(":memory:")).expect("mem db")
    }

    #[test]
    fn 坏库隔离重建_好库原样通过() {
        let dir = std::env::temp_dir().join(format!("zap-dbtest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("zap.db");

        // 好库：写入后 resilient 重开，数据原样、不产生 broken 文件
        {
            let conn = open_db(&db).unwrap();
            conn.execute(
                "INSERT INTO clipboard_history(content, created_at) VALUES('存活样本', 1)",
                [],
            )
            .unwrap();
        }
        let conn = open_db_resilient(&db).expect("好库应直通");
        let n: i64 = conn
            .query_row("SELECT count(*) FROM clipboard_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "好库数据不受自愈路径影响");
        drop(conn);
        let broken: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".broken-"))
            .collect();
        assert!(broken.is_empty(), "好库不得触发隔离");

        // 坏库：非 SQLite 字节 → open 失败 → 隔离改名 + 空库重建
        std::fs::write(&db, b"this is definitely not a sqlite database").unwrap();
        let conn = open_db_resilient(&db).expect("坏库应被隔离并重建空库");
        let n: i64 = conn
            .query_row("SELECT count(*) FROM clipboard_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "重建后是空库");
        drop(conn);
        let broken: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("zap.db.broken-"))
            .collect();
        assert_eq!(broken.len(), 1, "坏库现场应保留一份 {broken:?}");
        let kept = std::fs::read(dir.join(&broken[0])).unwrap();
        assert_eq!(
            kept, b"this is definitely not a sqlite database",
            "现场字节原样"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 迁移器补列幂等() {
        let dir = std::env::temp_dir().join(format!("zap-migtest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("zap.db");
        // 造 v0 旧库：无 pinned 列、user_version=0、有存量数据
        {
            let raw = rusqlite::Connection::open(&db).unwrap();
            raw.execute_batch(
                "CREATE TABLE clipboard_history (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     content TEXT NOT NULL UNIQUE,
                     created_at INTEGER NOT NULL);
                 INSERT INTO clipboard_history(content, created_at) VALUES('旧数据', 1);
                 PRAGMA user_version=0;",
            )
            .unwrap();
        }
        // open_db_resilient 过迁移：pinned 补上、版本写回、旧数据保留
        let conn = open_db_resilient(&db).expect("v0 旧库应平滑迁移");
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 1, "版本写回");
        let pinned: i64 = conn
            .query_row("SELECT pinned FROM clipboard_history WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pinned, 0, "旧数据默认未固定");
        drop(conn);
        // 幂等：重开不再执行步骤、不炸
        let conn2 = open_db_resilient(&db).expect("重开幂等");
        let v2: i64 = conn2
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v2, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dibv5字节布局与行序翻转() {
        // 2×2 图：row0 = 红(255,0,0)+半透明蓝(0,0,255)，row1 = 绿+灰
        let rgba: Vec<u8> = vec![
            255, 0, 0, 255, 0, 0, 255, 128, // row0
            0, 255, 0, 255, 100, 100, 100, 255, // row1
        ];
        let out = build_dibv5(2, 2, &rgba);
        assert_eq!(out.len(), 124 + 16, "BITMAPV5HEADER 124B + 16B 像素");
        let u32le = |o: usize| u32::from_le_bytes(out[o..o + 4].try_into().unwrap());
        let u16le = |o: usize| u16::from_le_bytes(out[o..o + 2].try_into().unwrap());
        assert_eq!(u32le(0), 124, "bV5Size");
        assert_eq!(u32le(4), 2, "bV5Width");
        assert_eq!(u32le(8), 2, "bV5Height 正值=bottom-up（负值 Word 粘不进）");
        assert_eq!(u16le(12), 1, "bV5Planes");
        assert_eq!(u16le(14), 32, "bV5BitCount");
        assert_eq!(u32le(16), 3, "bV5Compression=BI_BITFIELDS");
        assert_eq!(u32le(40), 0x00ff_0000, "RedMask");
        assert_eq!(u32le(52), 0xff00_0000, "AlphaMask 保透明通道");
        // 像素：bottom-up（row1 在前）+ RGBA→BGRA
        assert_eq!(&out[124..132], &[0, 255, 0, 255, 100, 100, 100, 255]);
        assert_eq!(&out[132..140], &[0, 0, 255, 255, 255, 0, 0, 128]);
        // 尺寸与数据不符拒绝
        assert!(set_image_dib(2, 2, &[0u8; 15]).is_err());
        assert!(set_image_dib(0, 0, &[]).is_err());
    }

    fn insert(conn: &Connection, content: &str, created_at: i64) -> i64 {
        conn.execute(
            "INSERT INTO clipboard_history(content, created_at) VALUES(?1, ?2)",
            params![content, created_at],
        )
        .expect("insert");
        conn.query_row("SELECT last_insert_rowid()", [], |r| r.get(0))
            .expect("rowid")
    }

    /// 直插图片表（跳过 PNG 编码：本测试只关心搜索召回，缩略图/全图用占位 BLOB）
    fn insert_image(conn: &Connection, w: i64, h: i64, created_at: i64) {
        conn.execute(
            "INSERT INTO clipboard_images(sha256, width, height, size, data, thumb, created_at)
             VALUES('testhash' || ?1 || ?2, ?1, ?2, 8, X'00', X'00', ?3)",
            params![w, h, created_at],
        )
        .expect("insert image");
    }

    /// 图片条目参与文本搜索：标题含尺寸，按尺寸可召回；打分不中不混入
    #[test]
    fn 图片条目参与文本搜索() {
        let conn = mem_db();
        insert(&conn, "普通文本条目", now_ms());
        insert_image(&conn, 1920, 1080, now_ms());
        // 按尺寸召回
        let items = search(&conn, "1920", 6);
        assert!(
            items
                .iter()
                .any(|i| i.kind == "clipimg" && i.title.contains("1920×1080")),
            "按尺寸应召回图片：{:?}",
            items.iter().map(|i| &i.title).collect::<Vec<_>>()
        );
        // 按尺寸宽高拆词也可召回（分词打分）
        let items = search(&conn, "1080", 6);
        assert!(items.iter().any(|i| i.kind == "clipimg"));
        // 打分不中不混入
        let items = search(&conn, "完全不匹配", 6);
        assert!(items.iter().all(|i| i.kind != "clipimg"));
    }

    /// 图片入库队列 worker：投递任务 → channel 断开 → 归还连接后图片已在库。
    /// 回调线程只取 RGBA 丢队列的拓扑由本测试锁定（编码/哈希/缩略图在 worker 侧）
    #[test]
    fn 图片队列worker入库() {
        let conn = mem_db();
        let (tx, rx) = std::sync::mpsc::sync_channel::<ImageTask>(4);
        let rgba: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        tx.send(ImageTask {
            rgba: rgba.clone(),
            width: 2,
            height: 2,
        })
        .expect("send");
        drop(tx);
        let conn = image_store_loop(conn, rx);
        let items = super::super::clipimage::search_recent(&conn, 10);
        assert_eq!(items.len(), 1, "worker 应把队列里的图片入库");
        assert!(items[0].2.id.starts_with("clipimg:"));
    }

    /// 线协议契约：clip 条目 target = 行号字符串（与 clipimg 同构），全文不随
    /// 列表过 IPC——回贴/仅复制由 open_item/secondary_action 按行号回查。
    /// 标题取首行、副标题带换行数，均不依赖全文
    #[test]
    fn 列表条目target为行号() {
        let conn = mem_db();
        let id = insert(&conn, "first line\nsecond line\nthird line", now_ms());
        let items = search(&conn, "", 6);
        let it = items
            .iter()
            .find(|i| i.id == format!("clip:{id}"))
            .expect("空查询应含该条");
        assert_eq!(it.target, id.to_string(), "target 应为行号");
        assert_eq!(it.title, "first line");
        assert!(
            it.subtitle.contains("还有 2 行"),
            "副标题应含换行数：{}",
            it.subtitle
        );
        // 打分路径同契约
        let items = search(&conn, "first", 6);
        assert_eq!(items[0].target, id.to_string());
        assert_eq!(items[0].title, "first line");
    }

    /// 按行号取全文（合并粘贴 clip_texts 的根基）：存在给原文，已被清理给 None
    #[test]
    fn 按行号取全文命中与缺失() {
        let conn = mem_db();
        let a = insert(&conn, "第一条", 1000);
        assert_eq!(get_by_id(&conn, a).as_deref(), Some("第一条"));
        assert_eq!(get_by_id(&conn, 99999), None, "不存在的行号给 None");
    }

    #[test]
    fn 重抄旧内容后最新一条按时间取() {
        let conn = mem_db();
        insert(&conn, "旧内容", 1000);
        insert(&conn, "新内容", 2000);
        // 模拟监听线程的 upsert：重抄「旧内容」只更新时间，行号不变
        conn.execute(
            "INSERT INTO clipboard_history(content, created_at) VALUES(?1, ?2)
             ON CONFLICT(content) DO UPDATE SET created_at=excluded.created_at",
            params!["旧内容", 3000],
        )
        .unwrap();
        let (id, text) = latest(&conn).expect("有历史");
        assert_eq!(text, "旧内容", "最新一条应是刚重抄的内容");
        assert_eq!(id, 1, "行号保持首次插入的 id，不能按 id 取成新内容");
    }

    #[test]
    fn 固定条目空查询置顶且带标记() {
        let conn = mem_db();
        let old = insert(&conn, "旧内容", 1000);
        let new = insert(&conn, "新内容", 2000);
        pin_toggle(&conn, old).unwrap();

        let items = search(&conn, "", 5);
        assert_eq!(items[0].id, format!("clip:{old}"), "固定条目应排最前");
        assert!(items[0].pinned, "固定条目应带结构化置顶标记");
        assert_eq!(items[0].title, "旧内容");
        assert!(items[1].id.ends_with(&new.to_string()), "其余按时间倒序");

        // 取消固定恢复时间序
        pin_toggle(&conn, old).unwrap();
        let items = search(&conn, "", 5);
        assert_eq!(items[0].id, format!("clip:{new}"));
    }

    #[test]
    fn 保留清理不删固定条目() {
        let conn = mem_db();
        let pinned = insert(&conn, "常用片段", 1000);
        insert(&conn, "旧记录", 2000);
        insert(&conn, "新记录", 3000);
        pin_toggle(&conn, pinned).unwrap();

        // 与监听线程相同的保留清理（LIMIT 1 只留最新一条）：固定条目应存活
        conn.execute(
            "DELETE FROM clipboard_history WHERE pinned=0 AND id NOT IN (
                 SELECT id FROM clipboard_history ORDER BY created_at DESC LIMIT 1
             )",
            [],
        )
        .unwrap();
        assert!(get_by_id(&conn, pinned).is_some(), "固定条目不被清理");
        assert!(get_by_id(&conn, 2).is_none(), "未固定的旧条目被清理");
    }

    #[test]
    fn 评分只看前256字符() {
        let conn = mem_db();
        // 关键词完整落在前 256 字符内（241~249 位）
        let mut long = "x".repeat(300);
        long.insert_str(240, " needles ");
        insert(&conn, &long, 1000);
        assert_eq!(search(&conn, "needle", 5).len(), 1);

        // 关键字落在 256 字符之外：与旧行为一致不参与匹配（全文仍随 target 返回）
        let mut beyond = "y".repeat(300);
        beyond.insert_str(280, "needle");
        insert(&conn, &beyond, 2000);
        assert_eq!(search(&conn, "needle", 5).len(), 1);
    }
}
