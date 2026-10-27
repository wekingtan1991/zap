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
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS clipboard_history (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             content TEXT NOT NULL UNIQUE,
             created_at INTEGER NOT NULL
         );",
    )?;
    // 旧库迁移：补 pinned 列（已存在时报错可忽略）
    let _ = conn.execute_batch(
        "ALTER TABLE clipboard_history ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
    );
    super::clipimage::init_table(&conn);
    Ok(conn)
}

/// 启动剪贴板监听线程：独立消息循环 + 独立写连接（WAL 下与读连接并存）
pub fn spawn_watcher(db_path: PathBuf) {
    std::thread::spawn(move || {
        let conn = match open_db(&db_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("zap: 剪贴板数据库打开失败: {e}");
                return;
            }
        };
        struct Handler {
            conn: Connection,
            cb: arboard::Clipboard,
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
                    // 无文本可取时尝试图片（截图类）。入库失败静默（超限/编码异常不打扰）
                    let _ = super::clipimage::store(
                        &self.conn,
                        &img.bytes,
                        img.width as u32,
                        img.height as u32,
                    );
                }
                CallbackResult::Next
            }

            fn on_clipboard_error(&mut self, error: io::Error) -> CallbackResult {
                eprintln!("zap: 剪贴板监听错误: {error}");
                CallbackResult::Next
            }
        }

        let handler = Handler {
            conn,
            cb: match arboard::Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("zap: 剪贴板初始化失败: {e}");
                    return;
                }
            },
        };
        match Master::new(handler) {
            Ok(mut master) => {
                if let Err(e) = master.run() {
                    eprintln!("zap: 剪贴板监听退出: {e}");
                }
            }
            Err(e) => eprintln!("zap: 剪贴板监听启动失败: {e}"),
        }
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

/// 搜索历史：Rust 侧统一打分（含拼音）。空查询返回最近条目（固定置顶，呼出即见常用）。
/// 评分只吃前 256 字符且由 SQL 侧 substr 截断——长文本（几 MB 的粘贴）不再每击键
/// 整行拖回内存，命中后再按行号点查全文
pub fn search(conn: &Connection, query: &str, limit: usize) -> Vec<SearchItem> {
    let now = now_ms();
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, created_at, pinned, substr(content, 1, 256) FROM clipboard_history
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
        ))
    }) else {
        return Vec::new();
    };
    let rows: Vec<(i64, i64, i64, String)> = rows.flatten().collect();

    if query.is_empty() {
        // 图片与文本条目按 固定 > 新旧 合并（两表各自已按该序取回）；
        // 图片上限取一半，避免连串截图挤掉文本常用项
        let mut merged: Vec<(i64, i64, SearchItem)> = rows
            .iter()
            .take(limit)
            .filter_map(|(id, t, pinned, _)| {
                let content = get_by_id(conn, *id)?;
                Some((
                    *pinned,
                    *t,
                    build_item(*id, content, *t, now, 0, *pinned == 1),
                ))
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
    let mut scored: Vec<(f32, i64, i64, i64)> = rows
        .iter()
        .filter_map(|(id, t, pinned, head)| {
            super::score_lowered(head, &q).map(|s| (s, *id, *pinned, *t))
        })
        .collect();
    // 同分档内固定优先，其次新的在前（与账号库口径一致）
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap()
            .then(b.2.cmp(&a.2))
            .then(b.3.cmp(&a.3))
    });
    scored.truncate(limit);
    scored
        .into_iter()
        .enumerate()
        .filter_map(|(i, (_, id, pinned, t))| {
            let content = get_by_id(conn, id)?;
            Some(build_item(id, content, t, now, i, pinned == 1))
        })
        .collect()
}

fn build_item(
    rowid: i64,
    content: String,
    created_at: i64,
    now: i64,
    index: usize,
    pinned: bool,
) -> SearchItem {
    let first_line = content.lines().next().unwrap_or("").trim().to_string();
    let title = if first_line.chars().count() > 60 {
        format!("{}…", first_line.chars().take(60).collect::<String>())
    } else if first_line.is_empty() {
        "(空白)".into()
    } else {
        first_line
    };
    let extra_lines = content.lines().count().saturating_sub(1);
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
        target: content,
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

    fn insert(conn: &Connection, content: &str, created_at: i64) -> i64 {
        conn.execute(
            "INSERT INTO clipboard_history(content, created_at) VALUES(?1, ?2)",
            params![content, created_at],
        )
        .expect("insert");
        conn.query_row("SELECT last_insert_rowid()", [], |r| r.get(0))
            .expect("rowid")
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
