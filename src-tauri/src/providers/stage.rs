//! 中转站：文件/文件夹的路径引用暂存区（存引用不存副本）。
//! 两层结构 = 临时（可被容量淘汰、可一键清空）+ 固定（永不清出，置顶展示），
//! 与剪贴板历史的「容量让位于固定」同一套行为模式。
//! 死链不自动删：列表时点查存在性，前端灰显 + 右键「重新定位」重绑路径。
//! 入口 = 声明化入口词（entry_hit），条目 kind=stage，激活切到前端 StageView。
//! 锁纪律：所有磁盘 IO（lnk 解析、存在性 metadata、图标提取）一律在共享库锁外
//! 进行——网络路径的 stat 可达秒级，进锁会拖死共用连接的 vault 与设置操作

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

use rusqlite::{Connection, params};
use serde::Serialize;

use super::SearchItem;

/// 未固定条目上限（设置可调，10–500）：中转站的价值在「一屏扫完」，
/// 超出挤掉最旧的未固定条目；固定条目不占容量
pub static CAP: AtomicI64 = AtomicI64::new(50);

/// 运行时开关：启动从 appconfig 加载、设置保存时同步（meta.state 由引擎装配）
pub static STAGE_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageRecord {
    pub id: i64,
    pub path: String,
    /// 展示名（路径末段；根盘符回退为路径本身）
    pub name: String,
    pub is_dir: bool,
    pub pinned: bool,
    /// 路径已失效（原文件/文件夹不存在）：列表时点查，不自动删
    pub missing: bool,
    /// 文件图标 data URL（PNG，按扩展名缓存）；失效条目不提取
    pub icon: Option<String>,
    pub added_at: i64,
}

/// 批量放入结果：added = 新增条数（重复放入计 0，仅置顶）；
/// failed = 未成功放入的路径（路径不存在等），不因个别失败拖垮整批
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageAddOutcome {
    pub added: i64,
    pub total: i64,
    pub failed: Vec<String>,
}

pub fn init_table(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS stage_items (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             path TEXT NOT NULL UNIQUE,
             is_dir INTEGER NOT NULL DEFAULT 0,
             pinned INTEGER NOT NULL DEFAULT 0,
             added_at INTEGER NOT NULL
         );",
    );
}

/// 归一化入参：去首尾空白与包裹引号（「复制文件地址」式粘贴的防御）。
/// .lnk 指针解析为真实目标（recent 条目放进来的应暂存本体而非快捷方式）
fn normalize(raw: &str) -> Result<String, String> {
    let p = raw.trim().trim_matches('"').trim();
    if p.is_empty() {
        return Err("路径为空".into());
    }
    if p.starts_with("shell:") {
        return Err("商店应用不支持放入中转站".into());
    }
    let p = if p.to_lowercase().ends_with(".lnk") {
        super::apps::resolve_lnk_target(p).unwrap_or_else(|| p.to_string())
    } else {
        p.to_string()
    };
    Ok(p)
}

/// 锁外解析单条：归一化 + 存在性 metadata（磁盘 IO 不持锁）
fn resolve_one(raw: &str) -> Result<(String, bool), String> {
    let p = normalize(raw)?;
    let meta = std::fs::metadata(&p).map_err(|_| format!("路径不存在：{p}"))?;
    Ok((p, meta.is_dir()))
}

/// 批量解析待放入路径（锁外执行，调用方随后只持锁写库）
pub fn resolve_paths(paths: &[String]) -> Vec<Result<(String, bool), String>> {
    paths.iter().map(|p| resolve_one(p)).collect()
}

/// 锁内提交放入：逐条入库（个别失败不影响其余），返回新增数/总数/失败明细
pub fn commit_add(
    conn: &Connection,
    resolved: Vec<Result<(String, bool), String>>,
) -> StageAddOutcome {
    let mut outcome = StageAddOutcome {
        added: 0,
        total: 0,
        failed: Vec::new(),
    };
    for r in resolved {
        match r {
            Ok((p, is_dir)) => match insert_one(conn, &p, is_dir) {
                Ok(is_new) => outcome.added += i64::from(is_new),
                Err(e) => outcome.failed.push(e),
            },
            Err(e) => outcome.failed.push(e),
        }
    }
    outcome.total = count(conn);
    outcome
}

/// 锁内入库一条：返回是否新增。重复放入 = 严格置顶（取「库内最大 added_at + 1」，
/// 与同毫秒新插入拉开差距），保留既有固定态（重复 = 再次置顶，不是取消固定）
fn insert_one(conn: &Connection, p: &str, is_dir: bool) -> Result<bool, String> {
    let existed: bool = conn
        .query_row(
            "SELECT 1 FROM stage_items WHERE path=?1",
            params![p],
            |_| Ok(true),
        )
        .unwrap_or(false);
    conn.execute(
        "INSERT INTO stage_items(path, is_dir, pinned, added_at) VALUES(?1, ?2, 0, ?3)
         ON CONFLICT(path) DO UPDATE SET
             added_at=(SELECT COALESCE(MAX(added_at),0) FROM stage_items)+1",
        params![p, is_dir, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    evict_beyond_cap(conn);
    Ok(!existed)
}

/// 未固定条目超出容量时挤掉最旧的（固定条目不参与，同剪贴板清理口径）。
/// added_at 同毫秒时 id DESC 兜底确定性
fn evict_beyond_cap(conn: &Connection) {
    let _ = conn.execute(
        "DELETE FROM stage_items WHERE pinned=0 AND id NOT IN (
             SELECT id FROM stage_items WHERE pinned=0 ORDER BY added_at DESC, id DESC LIMIT ?1
         )",
        params![CAP.load(Ordering::Relaxed).max(1)],
    );
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM stage_items", [], |r| r.get(0))
        .unwrap_or(0)
}

/// 锁内取行：纯库操作，不做存在性/图标（磁盘 IO 出锁，见模块头锁纪律）
pub struct StageRowData {
    pub id: i64,
    pub path: String,
    pub is_dir: bool,
    pub pinned: bool,
    pub added_at: i64,
}

pub fn rows(conn: &Connection) -> Vec<StageRowData> {
    let mut stmt = match conn.prepare(
        "SELECT id, path, is_dir, pinned, added_at FROM stage_items
         ORDER BY pinned DESC, added_at DESC, id DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mapped = match stmt.query_map([], |r| {
        Ok(StageRowData {
            id: r.get(0)?,
            path: r.get(1)?,
            is_dir: r.get::<_, i64>(2)? != 0,
            pinned: r.get::<_, i64>(3)? != 0,
            added_at: r.get(4)?,
        })
    }) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };
    mapped.flatten().collect()
}

/// 锁外补全：存在性点查（symlink 目标失效时 metadata 兜不住，exists 走真实解析）
/// + 图标。missing 现场判定、不自动删
pub fn finish(rows: Vec<StageRowData>) -> Vec<StageRecord> {
    rows.into_iter()
        .map(|r| {
            let missing = !std::path::Path::new(&r.path).exists();
            let name = std::path::Path::new(&r.path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| r.path.clone());
            let icon = if missing {
                None
            } else {
                icon_for(&r.path, r.is_dir)
            };
            StageRecord {
                id: r.id,
                name,
                path: r.path,
                is_dir: r.is_dir,
                pinned: r.pinned,
                missing,
                icon,
                added_at: r.added_at,
            }
        })
        .collect()
}

/// 图标缓存：文件图标只由扩展名决定（目录统一一份），按扩展名惰性提取——
/// 几十个扩展名即覆盖全部条目，远轻于按路径缓存
static ICON_CACHE: std::sync::LazyLock<Mutex<HashMap<String, Option<String>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// 缓存键：目录固定 "/"，与无扩展名文件不共用
fn icon_cache_key(path: &str, is_dir: bool) -> String {
    if is_dir {
        return "/".to_string();
    }
    std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// 提取失败也缓存 None：同扩展名不反复打系统调用
fn icon_for(path: &str, is_dir: bool) -> Option<String> {
    let key = icon_cache_key(path, is_dir);
    if let Ok(cache) = ICON_CACHE.lock()
        && let Some(hit) = cache.get(&key)
    {
        return hit.clone();
    }
    let icon = super::apps::extract_icon_png(path).map(|png| super::apps::to_data_url(&png));
    if let Ok(mut cache) = ICON_CACHE.lock() {
        cache.insert(key, icon.clone());
    }
    icon
}

/// 移出中转站（只删引用，不碰原文件）
pub fn remove(conn: &Connection, id: i64) -> Result<(), String> {
    let n = conn
        .execute("DELETE FROM stage_items WHERE id=?1", params![id])
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("条目不存在".into());
    }
    Ok(())
}

/// 清空临时区（固定条目保留），返回清掉条数
pub fn clear(conn: &Connection) -> Result<usize, String> {
    conn.execute("DELETE FROM stage_items WHERE pinned=0", [])
        .map_err(|e| e.to_string())
}

/// 固定/取消固定，返回操作后的状态
pub fn pin_toggle(conn: &Connection, id: i64) -> Result<bool, String> {
    let cur: i64 = conn
        .query_row(
            "SELECT pinned FROM stage_items WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|_| "条目不存在".to_string())?;
    let next = if cur == 0 { 1 } else { 0 };
    conn.execute(
        "UPDATE stage_items SET pinned=?1 WHERE id=?2",
        params![next, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(next != 0)
}

/// 锁外解析重定位目标（normalize + 存在性，磁盘 IO 不持锁）
pub fn resolve_relocate(raw: &str) -> Result<(String, bool), String> {
    resolve_one(raw)
}

/// 锁内提交重定位：换绑路径（保留固定态与加入时间）
pub fn commit_relocate(conn: &Connection, id: i64, path: &str, is_dir: bool) -> Result<(), String> {
    let n = conn
        .execute(
            "UPDATE stage_items SET path=?1, is_dir=?2 WHERE id=?3",
            params![path, is_dir as i64, id],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("条目不存在".into());
    }
    Ok(())
}

/// 撤销恢复：原样插回刚移出/清空的条目（含固定态与时间戳，排序位不变）。
/// 不走 normalize/metadata——恢复的是自家刚删掉的数据；路径已被重新放入时
/// 幂等忽略；刻意不触发容量淘汰（恢复不应立刻挤出条目，上限在下次放入时收敛）
pub fn restore(
    conn: &Connection,
    path: &str,
    is_dir: bool,
    pinned: bool,
    added_at: i64,
) -> Result<bool, String> {
    let n = conn
        .execute(
            "INSERT OR IGNORE INTO stage_items(path, is_dir, pinned, added_at) VALUES(?1, ?2, ?3, ?4)",
            params![path, is_dir as i64, pinned as i64, added_at],
        )
        .map_err(|e| e.to_string())?;
    Ok(n > 0)
}

/// 复制文件到剪贴板（CF_HDROP）：粘贴到任意对话框/资源管理器即为复制原文件。
/// 中转站「携带」语义的关键动作——arboard 不支持文件列表，直接写 Win32 剪贴板
#[cfg(windows)]
pub fn copy_files_to_clipboard(paths: &[String]) -> Result<(), String> {
    use windows::Win32::Foundation::{GlobalFree, HANDLE};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::UI::Shell::DROPFILES;
    // CF_HDROP = 15（winuser.h 的剪贴板格式常量，避免为常量引 Ole 特征）
    const CF_HDROP: u32 = 15;

    if paths.is_empty() {
        return Err("没有可复制的文件".into());
    }
    // DROPFILES 头 + 双 NUL 终止的宽字符路径串
    let mut wide: Vec<u16> = Vec::new();
    for p in paths {
        wide.extend(p.encode_utf16());
        wide.push(0);
    }
    wide.push(0);

    unsafe {
        OpenClipboard(None).map_err(|e| format!("打开剪贴板失败：{e}"))?;
        let result = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| format!("清空剪贴板失败：{e}"))?;
            let header = std::mem::size_of::<DROPFILES>();
            let bytes = header + wide.len() * 2;
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes)
                .map_err(|e| format!("分配剪贴板内存失败：{e}"))?;
            let ptr = GlobalLock(h);
            if ptr.is_null() {
                let _ = GlobalFree(Some(h));
                return Err("锁定剪贴板内存失败".into());
            }
            let df = DROPFILES {
                pFiles: header as u32,
                fWide: windows::core::BOOL(1),
                ..Default::default()
            };
            let base = ptr as *mut u8;
            std::ptr::copy_nonoverlapping(&df as *const DROPFILES as *const u8, base, header);
            std::ptr::copy_nonoverlapping(
                wide.as_ptr() as *const u8,
                base.add(header),
                wide.len() * 2,
            );
            let _ = GlobalUnlock(h);
            // 成功后内存归系统所有，不得 GlobalFree
            SetClipboardData(CF_HDROP, Some(HANDLE(h.0))).map_err(|e| {
                let _ = GlobalFree(Some(h));
                format!("写入剪贴板失败：{e}")
            })?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

#[cfg(not(windows))]
pub fn copy_files_to_clipboard(_paths: &[String]) -> Result<(), String> {
    Err("仅支持 Windows".into())
}

pub struct StagePlugin;

impl StagePlugin {
    /// 入口条目：搜索命中与兜底清单共用同一构造，文案不漂移
    fn entry_item() -> SearchItem {
        SearchItem {
            id: "stage:entry".into(),
            title: "打开中转站".into(),
            subtitle: "拖入文件/文件夹随手暂存 · 点击打开 · 常用可固定".into(),
            kind: "stage".into(),
            target: "entry".into(),
            icon: None,
            score: 0.0,
        }
    }
}

impl super::plugin::Plugin for StagePlugin {
    fn search(&self, query: &str, _conn: Option<&Connection>) -> Vec<SearchItem> {
        let words = self.meta().map(|m| m.entries).unwrap_or(&[]);
        if !STAGE_ON.load(Ordering::Relaxed) || !super::plugin::entry_hit(query, words) {
            return Vec::new();
        }
        vec![Self::entry_item()]
    }

    fn fallback_entry(&self) -> Vec<SearchItem> {
        if STAGE_ON.load(Ordering::Relaxed) {
            vec![Self::entry_item()]
        } else {
            Vec::new()
        }
    }

    fn meta(&self) -> Option<super::plugin::PluginMeta> {
        Some(super::plugin::PluginMeta {
            id: "stage",
            title: "中转站",
            hint: "拖入文件/文件夹随手暂存：点击打开、右键操作，常用文件夹可固定不被清出",
            key: "plugin_stage",
            default_on: true,
            state: &STAGE_ON,
            entries: &["中转站", "暂存", "stage", "zzz"],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_table(&conn);
        conn
    }

    /// 测试用临时文件（存在性是 resolve 的硬校验）
    fn temp_file(tag: &str) -> String {
        let p =
            std::env::temp_dir().join(format!("zap-stage-test-{}-{}.txt", std::process::id(), tag));
        std::fs::write(&p, b"x").unwrap();
        p.to_string_lossy().into_owned()
    }

    fn temp_dir(tag: &str) -> String {
        let p =
            std::env::temp_dir().join(format!("zap-stage-test-dir-{}-{}", std::process::id(), tag));
        let _ = std::fs::create_dir_all(&p);
        p.to_string_lossy().into_owned()
    }

    /// 测试装配：resolve + commit 两段（生产走引擎同名拆分）
    fn add(conn: &Connection, paths: &[String]) -> StageAddOutcome {
        commit_add(conn, resolve_paths(paths))
    }

    /// 测试装配：rows + finish
    fn list(conn: &Connection) -> Vec<StageRecord> {
        finish(rows(conn))
    }

    #[test]
    fn 放入_列表与去重置顶() {
        let conn = mem_conn();
        let a = temp_file("a");
        let b = temp_file("b");
        let out = add(&conn, &[a.clone(), b.clone()]);
        assert_eq!(out.added, 2);
        assert_eq!(out.total, 2);
        assert!(out.failed.is_empty());
        // 重复放入：计 0 新增但置顶（b 现在在 a 前）
        let out = add(&conn, std::slice::from_ref(&a));
        assert_eq!(out.added, 0);
        assert_eq!(out.total, 2);
        let rows = list(&conn);
        assert_eq!(rows[0].path, a, "重复放入应置顶");
        // 不存在与 shell: 拒绝
        let out = add(&conn, &["C:\\zap-不存在-xyz".into()]);
        assert_eq!(out.added, 0);
        assert_eq!(out.failed.len(), 1);
        let out = add(&conn, &["shell:AppsFolder\\x".into()]);
        assert!(out.failed[0].contains("商店应用"));
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
    }

    #[test]
    fn 容量淘汰且固定豁免() {
        let conn = mem_conn();
        CAP.store(2, Ordering::Relaxed);
        let a = temp_file("c1");
        let b = temp_file("c2");
        let c = temp_file("c3");
        let d = temp_file("c4");
        // 先放 a、b 并固定 b（较旧的那条），再放 c：未固定只有 a、c，
        // 不超容量，但总数 3 > cap——固定条目不占容量
        add(&conn, &[a.clone(), b.clone()]);
        let id_b = list(&conn).iter().find(|r| r.path == b).unwrap().id;
        assert!(pin_toggle(&conn, id_b).unwrap());
        add(&conn, std::slice::from_ref(&c));
        assert_eq!(count(&conn), 3, "未固定 2 + 固定 1");
        // 再放 d：未固定超 2，被挤掉的是最旧的未固定 a，b 豁免
        add(&conn, std::slice::from_ref(&d));
        let paths: Vec<String> = list(&conn).into_iter().map(|r| r.path).collect();
        assert_eq!(
            paths,
            vec![b.clone(), d.clone(), c.clone()],
            "固定置顶且不被淘汰，未固定挤掉最旧 a"
        );
        CAP.store(50, Ordering::Relaxed);
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
        let _ = std::fs::remove_file(c);
        let _ = std::fs::remove_file(d);
    }

    #[test]
    fn 清空只清未固定_移出与重定位() {
        let conn = mem_conn();
        let a = temp_file("d1");
        let b = temp_dir("d2");
        add(&conn, &[a.clone(), b.clone()]);
        let rows = list(&conn);
        assert!(rows.iter().find(|r| r.path == b).unwrap().is_dir);
        let id_b = rows.iter().find(|r| r.path == b).unwrap().id;
        pin_toggle(&conn, id_b).unwrap();
        let cleared = clear(&conn).unwrap();
        assert_eq!(cleared, 1, "固定条目保留");
        assert_eq!(count(&conn), 1);
        // 重定位：换绑到另一真实路径，固定态保留（resolve 锁外 + commit 锁内）
        let c = temp_file("d3");
        let (p, is_dir) = resolve_relocate(&c).unwrap();
        commit_relocate(&conn, id_b, &p, is_dir).unwrap();
        let rows = list(&conn);
        assert_eq!(rows[0].path, c);
        assert!(rows[0].pinned);
        // 移出
        remove(&conn, rows[0].id).unwrap();
        assert!(remove(&conn, 9999).is_err(), "移出不存在的条目应报错");
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_dir_all(b);
        let _ = std::fs::remove_file(c);
    }

    #[test]
    fn 撤销恢复原样插回且幂等() {
        let conn = mem_conn();
        let a = temp_file("r1");
        let b = temp_file("r2");
        add(&conn, &[a.clone(), b.clone()]);
        let rows = list(&conn);
        let b_row = rows.iter().find(|r| r.path == b).unwrap().clone();
        remove(&conn, b_row.id).unwrap();
        assert_eq!(count(&conn), 1);
        // 原样插回：id 会变，但时间戳/固定态/排序位不变
        assert!(
            restore(
                &conn,
                &b_row.path,
                b_row.is_dir,
                b_row.pinned,
                b_row.added_at
            )
            .unwrap()
        );
        let rows = list(&conn);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, b_row.path, "撤销恢复后回到原排序位");
        assert!(!rows[0].pinned);
        // 路径已在库中（用户重新放入过）：幂等忽略
        assert!(!restore(&conn, &b_row.path, false, false, 0).unwrap());
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
    }

    #[test]
    fn 死链不自动删标记失效() {
        let conn = mem_conn();
        let a = temp_file("e1");
        add(&conn, std::slice::from_ref(&a));
        assert!(!list(&conn)[0].missing);
        let _ = std::fs::remove_file(&a);
        let rows = list(&conn);
        assert_eq!(rows.len(), 1, "死链保留在列表");
        assert!(rows[0].missing);
        assert!(rows[0].icon.is_none(), "失效条目不提取图标");
        remove(&conn, rows[0].id).unwrap();
    }

    #[test]
    fn 图标缓存键按扩展名归一() {
        assert_eq!(icon_cache_key("C:\\a\\b.TXT", false), "txt");
        assert_eq!(
            icon_cache_key("C:\\a\\b.tar.gz", false),
            "gz",
            "只取末段扩展名"
        );
        assert_eq!(icon_cache_key("C:\\a\\noext", false), "");
        assert_eq!(
            icon_cache_key("C:\\a\\dir", true),
            "/",
            "目录与无扩展名文件不可共用键"
        );
    }

    #[test]
    fn 入口词出条目_停用即隐() {
        use crate::providers::plugin::Plugin;
        let p = StagePlugin;
        let items = p.search("中转", None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "stage");
        assert_eq!(items[0].target, "entry");
        assert!(p.search("zz", None).len() == 1, "拼音式入口词也应命中");
        assert!(p.search("rust", None).is_empty(), "常规搜索不出入口条目");
        STAGE_ON.store(false, Ordering::Relaxed);
        assert!(p.search("中转", None).is_empty());
        STAGE_ON.store(true, Ordering::Relaxed);
    }
}
