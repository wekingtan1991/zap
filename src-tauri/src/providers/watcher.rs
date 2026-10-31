//! 剪贴板监听线程（自 clipboard.rs 拆出，审计 D3）：独立消息循环 + 独立写连接
//! 的变更回调、图片入库 worker（channel 有界队列）、watcher 自愈 respawn、
//! 密码管理器来源过滤。历史存储/插件适配/写入层（winclip）各归其位

use std::io;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use clipboard_master::{CallbackResult, ClipboardHandler, Master};
use rusqlite::{Connection, params};
use windows::Win32::System::DataExchange::GetClipboardOwner;

use super::clipboard::{CLIP_ENABLED, RETENTION_LIMIT, in_suppress_window};
use super::util::now_ms;
/// 单条文本入库硬上限（评审 P1-08）：一次超长粘贴（如整本书）会以单条
/// 抬升数 GB 库体积，且锁内 String 拷贝转嫁为主线程排队。超限拒收（诊断
/// 留痕），剪贴板本体不受影响——粘贴功能与历史收录解耦
pub const MAX_CLIP_TEXT_BYTES: usize = 1_000_000;

/// 入库判定（纯函数）：空文本与超限文本不进历史。来源过滤（密码管理器/
/// vault 密码集）在调用方（各自依赖系统 API 与连接）
fn should_store(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_CLIP_TEXT_BYTES
}

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
        let conn = match super::db::open_db_resilient(&worker_path) {
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
            let conn = match super::db::open_db_resilient(&db_path) {
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
                        // 单条硬上限（评审 P1-08）：超长粘贴拒收录（诊断留痕），
                        // 粘贴功能本身不受影响
                        if !should_store(&text) {
                            if !text.is_empty() {
                                super::diag::log_warn(&format!(
                                    "clip\t超长文本拒收录\t{} 字节",
                                    text.len()
                                ));
                            }
                            return CallbackResult::Next;
                        }
                        if !text.is_empty() && !is_from_password_manager() {
                            let now = now_ms();
                            // head/nl_count 物化在写路径（评审 P1-08）：SQL 侧
                            // 一次算好，读取路径不再对全文现算
                            let _ = self.conn.execute(
                                "INSERT INTO clipboard_history(content, created_at, head, nl_count)
                                 VALUES(?1, ?2, substr(?1, 1, 256),
                                        length(?1) - length(replace(?1, char(10), '')))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn mem_db() -> Connection {
        crate::providers::db::open_db(Path::new(":memory:")).expect("mem db")
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
}
