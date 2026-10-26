//! 中转站：文件/文件夹的路径引用暂存区（存引用不存副本）。
//! 两层结构 = 临时（可被容量淘汰、可一键清空）+ 固定（永不清出，置顶展示），
//! 与剪贴板历史的「容量让位于固定」同一套行为模式。
//! 死链不自动删：列表时点查存在性，前端灰显 + 右键「重新定位」重绑路径。
//! 入口 = 声明化入口词（entry_hit），条目 kind=stage，激活切到前端 StageView

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

/// 放入一条：存在性校验 → 去重置顶（UNIQUE 冲突仅提升 added_at）→ 容量淘汰。
/// 重复放入保留既有固定态（重复 = 再次置顶，不是取消固定）
pub fn add_one(conn: &Connection, raw: &str) -> Result<i64, String> {
    let p = normalize(raw)?;
    let meta = std::fs::metadata(&p).map_err(|_| format!("路径不存在：{p}"))?;
    let is_dir = meta.is_dir() as i64;
    let existed: bool = conn
        .query_row(
            "SELECT 1 FROM stage_items WHERE path=?1",
            params![p],
            |_| Ok(true),
        )
        .unwrap_or(false);
    conn.execute(
        // 重复放入 = 严格置顶：取「库内最大 added_at + 1」，与同毫秒新插入的
        // 条目拉开差距（新插入同为 now_ms 时由 id DESC 次序兜底新者在前）
        "INSERT INTO stage_items(path, is_dir, pinned, added_at) VALUES(?1, ?2, 0, ?3)
         ON CONFLICT(path) DO UPDATE SET
             added_at=(SELECT COALESCE(MAX(added_at),0) FROM stage_items)+1",
        params![p, is_dir, now_ms()],
    )
    .map_err(|e| e.to_string())?;
    evict_beyond_cap(conn);
    Ok(if existed { 0 } else { 1 })
}

/// 未固定条目超出容量时挤掉最旧的（固定条目不参与，同剪贴板清理口径）
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

/// 批量放入：逐条尝试，个别失败不影响其余；全部失败才整体报错
pub fn add(conn: &Connection, paths: &[String]) -> StageAddOutcome {
    let mut outcome = StageAddOutcome {
        added: 0,
        total: 0,
        failed: Vec::new(),
    };
    for p in paths {
        match add_one(conn, p) {
            Ok(n) => outcome.added += n,
            Err(e) => outcome.failed.push(e),
        }
    }
    outcome.total = count(conn);
    outcome
}

pub fn count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM stage_items", [], |r| r.get(0))
        .unwrap_or(0)
}

/// 列表：固定在上（按放入时间），其后临时按放入时间倒序。
/// missing 现场点查（symlink 目标失效时 metadata 兜不住，exists 走真实解析）
pub fn list(conn: &Connection) -> Vec<StageRecord> {
    let mut stmt = match conn.prepare(
        "SELECT id, path, is_dir, pinned FROM stage_items ORDER BY pinned DESC, added_at DESC, id DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    }) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    // query_map 迭代借用 stmt，先收行再做存在性点查
    let mut out = Vec::new();
    for row in rows.flatten() {
        let (id, path, is_dir, pinned) = row;
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let missing = !std::path::Path::new(&path).exists();
        out.push(StageRecord {
            id,
            name,
            path,
            is_dir: is_dir != 0,
            pinned: pinned != 0,
            missing,
        });
    }
    out
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

/// 重新定位：为失效条目重绑路径（保留固定态与加入时间）
pub fn relocate(conn: &Connection, id: i64, raw: &str) -> Result<(), String> {
    let p = normalize(raw)?;
    let meta = std::fs::metadata(&p).map_err(|_| format!("路径不存在：{p}"))?;
    let n = conn
        .execute(
            "UPDATE stage_items SET path=?1, is_dir=?2 WHERE id=?3",
            params![p, meta.is_dir() as i64, id],
        )
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("条目不存在".into());
    }
    Ok(())
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

impl super::plugin::Plugin for StagePlugin {
    fn search(&self, query: &str, _conn: Option<&Connection>) -> Vec<SearchItem> {
        let words = self.meta().map(|m| m.entries).unwrap_or(&[]);
        if !STAGE_ON.load(Ordering::Relaxed) || !super::plugin::entry_hit(query, words) {
            return Vec::new();
        }
        vec![SearchItem {
            id: "stage:entry".into(),
            title: "打开中转站".into(),
            subtitle: "拖入文件/文件夹随手暂存 · 双击打开 · 常用可固定".into(),
            kind: "stage".into(),
            target: "entry".into(),
            icon: None,
            score: 0.0,
        }]
    }

    fn meta(&self) -> Option<super::plugin::PluginMeta> {
        Some(super::plugin::PluginMeta {
            id: "stage",
            title: "中转站",
            hint: "拖入文件/文件夹随手暂存：双击打开、右键操作，常用文件夹可固定不被清出",
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

    /// 测试用临时文件（存在性是 add 的硬校验）
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
        let list = list(&conn);
        assert_eq!(list[0].path, a, "重复放入应置顶");
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
        // 重定位：换绑到另一真实路径，固定态保留
        let c = temp_file("d3");
        relocate(&conn, id_b, &c).unwrap();
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
    fn 死链不自动删标记失效() {
        let conn = mem_conn();
        let a = temp_file("e1");
        add(&conn, std::slice::from_ref(&a));
        assert!(!list(&conn)[0].missing);
        let _ = std::fs::remove_file(&a);
        let list = list(&conn);
        assert_eq!(list.len(), 1, "死链保留在列表");
        assert!(list[0].missing);
        remove(&conn, list[0].id).unwrap();
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
