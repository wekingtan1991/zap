//! 共享库的打开/迁移/坏库自愈（自 clipboard.rs 拆出，审计 D1）：open_db 是
//! 全库唯一连接工厂——clipboard_history 建表 + 五张业务表 init + schema 迁移
//! 全部收拢在此（原散落两处：engine 装配调 vault/appconfig/apps/stage 四个
//! init，clipimage 的 init 藏在 clipboard::open_db 内部）。打开即完整 schema，
//! 引擎/监听/图片 worker 三路连接口径一致

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::util::now_ms;

pub fn open_db(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        // busy_timeout 显式化（评审 P1-07）：rusqlite 默认已是 5000ms，但那是
        // 隐式依赖——显式设 1500ms 把「写-写冲突时最多等 1.5s」变成明文契约，
        // 宁可主线程命令快败重试，不让同步命令被 16MB 大图事务拖满 5 秒。
        // 放在 journal_mode 之前，让 WAL 设置本身也受保护。
        // synchronous=NORMAL 是 WAL 模式推荐值（评审 P1-08）：fsync 只在
        // checkpoint 时发生，断电最多丢最后事务、库不损坏
        "PRAGMA busy_timeout=1500;
         PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS clipboard_history (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             content TEXT NOT NULL UNIQUE,
             created_at INTEGER NOT NULL
         );",
    )?;
    init_all(&conn);
    if let Err(e) = migrate(&conn) {
        super::diag::log_err(&format!("db\tschema 迁移失败: {e}"));
    }
    Ok(conn)
}

/// 五张业务表建表收拢（审计 D1）：全部 IF NOT EXISTS 幂等，监听/worker 连接
/// 打开时顺带保证表齐——谁先启动都不缺表
fn init_all(conn: &Connection) {
    super::appconfig::init_table(conn);
    super::apps::init_stats(conn);
    super::clipimage::init_table(conn);
    super::stage::init_table(conn);
    super::vault::init_table(conn);
}

/// 当前 schema 版本：每次破坏性/加列演进 +1，对应 migrate 里的 `v < N` 步骤
const SCHEMA_VERSION: i64 = 2;

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
        if v < 2 {
            // 步骤 2（v1→v2，评审 P1-08）：head/nl_count 物化列。搜索评分的
            // substr 与换行数原来每击键对 top-300 全文现算（replace 全文扫描，
            // 把 clip 锁持有时长转嫁为主线程排队）；入库时写一次、读取改读列。
            // 回填对存量数据一次完成
            conn.execute_batch(
                "ALTER TABLE clipboard_history ADD COLUMN head TEXT NOT NULL DEFAULT '';
                 ALTER TABLE clipboard_history ADD COLUMN nl_count INTEGER NOT NULL DEFAULT 0;
                 UPDATE clipboard_history SET
                     head = substr(content, 1, 256),
                     nl_count = length(content) - length(replace(content, char(10), ''));",
            )
            .map_err(|e| format!("物化列迁移失败: {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(v, SCHEMA_VERSION, "版本写回");
        let pinned: i64 = conn
            .query_row("SELECT pinned FROM clipboard_history WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pinned, 0, "旧数据默认未固定");
        // 物化列回填（评审 P1-08）：存量数据的 head/nl_count 一次算好
        let (head, nl): (String, i64) = conn
            .query_row(
                "SELECT head, nl_count FROM clipboard_history WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(head, "旧数据");
        assert_eq!(nl, 0);
        drop(conn);
        // 幂等：重开不再执行步骤、不炸
        let conn2 = open_db_resilient(&db).expect("重开幂等");
        let v2: i64 = conn2
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v2, SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
