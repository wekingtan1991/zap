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

/// 剪贴板历史总开关（设置页/托盘切换），关闭时监听仍运行但不入库
pub static CLIP_ENABLED: AtomicBool = AtomicBool::new(true);

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
                        let _ = self.conn.execute(
                            "DELETE FROM clipboard_history WHERE id NOT IN (
                                 SELECT id FROM clipboard_history ORDER BY created_at DESC LIMIT ?1
                             )",
                            params![RETENTION_LIMIT.load(Ordering::Relaxed)],
                        );
                    }
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

/// 搜索历史：Rust 侧统一打分（含拼音）。空查询返回最近 5 条（打开面板即见最近复制）
pub fn search(conn: &Connection, query: &str, limit: usize) -> Vec<SearchItem> {
    let now = now_ms();
    let Ok(mut stmt) = conn.prepare(
        "SELECT content, created_at FROM clipboard_history
         ORDER BY created_at DESC LIMIT 300",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) else {
        return Vec::new();
    };
    let rows: Vec<(String, i64)> = rows.flatten().collect();

    if query.is_empty() {
        return rows
            .iter()
            .take(limit)
            .enumerate()
            .map(|(i, (c, t))| build_item(c.clone(), *t, now, i))
            .collect();
    }

    let q = query.trim().to_lowercase();
    let mut scored: Vec<(f32, String, i64)> = rows
        .iter()
        .filter_map(|(c, t)| {
            // 长内容截断参与拼音匹配；直接子串匹配仍覆盖全文
            let head: String = c.chars().take(256).collect();
            super::score_lowered(&head, &q).map(|s| (s, c.clone(), *t))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then_with(|| b.2.cmp(&a.2)));
    scored.truncate(limit);
    scored
        .into_iter()
        .enumerate()
        .map(|(i, (_, c, t))| build_item(c, t, now, i))
        .collect()
}

fn build_item(content: String, created_at: i64, now: i64, index: usize) -> SearchItem {
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
        id: format!("clip:{index}"),
        title,
        subtitle,
        kind: "clip".into(),
        target: content,
        icon: None,
        score: 60.0 - index as f32,
    }
}

fn ago(then: i64, now: i64) -> String {
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

/// 最新一条历史的 (行号, 内容)：面板呼出时内容识别的输入。
/// 无历史（首次使用/已清空）返回 None
pub fn latest(conn: &Connection) -> Option<(i64, String)> {
    conn.query_row(
        "SELECT id, content FROM clipboard_history ORDER BY id DESC LIMIT 1",
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

/// 剪贴板历史源插件适配：唯一 needs_conn 的常规源，
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
