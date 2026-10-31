use std::sync::atomic::AtomicBool;

use rusqlite::{Connection, params};

use super::SearchItem;
use super::util::now_ms;

// 写入层在 winclip.rs（审计 D2 拆出）：调用方 23 处经 clipboard::<sym> 引用，
// re-export 保持零改动
pub use super::winclip::{
    begin_self_copy, copy_text_private, in_suppress_window, retrying, set_image_dib,
};

/// 剪贴板历史总开关（设置页/托盘切换），关闭时监听仍运行但不入库
pub static CLIP_ENABLED: AtomicBool = AtomicBool::new(true);

/// 「选中即粘贴」开关：剪贴板条目选中后焦点交还原窗口并发送 Ctrl+V。
/// 关闭时退化为仅回填剪贴板
pub static PASTE_TO_PREV: AtomicBool = AtomicBool::new(true);

/// 历史保留条数（设置可调）
pub static RETENTION_LIMIT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(500);

/// 剪贴板设置三项的值对象（评审 P1-18 owner 链路）：load/apply 的载体，
/// 字段与 KEY_CLIP_* 三键一一对应
pub struct ClipSettings {
    pub enabled: bool,
    pub max: i64,
    pub autopaste: bool,
}

/// 未固定图片总字节预算（评审 P1-08）：500 条×16MB 理论上限 8GB 不设防，
/// 256MB 预算外从最旧删起。条数上限仍由 RETENTION_LIMIT 把守（双阈值）
pub const IMAGE_BUDGET_BYTES: i64 = 256 * 1024 * 1024;

/// 搜索历史：Rust 侧统一打分（含拼音）。空查询返回最近条目（固定置顶，呼出即见常用）；
/// 文本查询下图片条目按 标题+副标题（含尺寸）参与打分同池竞争。
/// 列表只吃前 256 字符（SQL 侧 substr）与换行数（SQL 侧现算）——全文不随列表过
/// IPC（几 MB 的粘贴曾每次呼出/每击键整行拖回），回贴/仅复制按行号回查
/// （open_item 与 secondary_action 的 clip 分支）
pub fn search(conn: &Connection, query: &str, limit: usize) -> Vec<SearchItem> {
    let now = now_ms();
    // head/nl_count 读物化列（评审 P1-08）：旧实现每击键对 top-300 全文跑
    // substr+replace 现算，把锁持有时长转嫁为主线程排队
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, created_at, pinned, head, nl_count
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
        crate::providers::db::open_db(Path::new(":memory:")).expect("mem db")
    }

    fn insert(conn: &Connection, content: &str, created_at: i64) -> i64 {
        conn.execute(
            // 与监听线程写路径同款：head/nl_count 物化（评审 P1-08）
            "INSERT INTO clipboard_history(content, created_at, head, nl_count)
             VALUES(?1, ?2, substr(?1, 1, 256), length(?1) - length(replace(?1, char(10), '')))",
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
            "INSERT INTO clipboard_history(content, created_at, head, nl_count)
             VALUES(?1, ?2, substr(?1, 1, 256), length(?1) - length(replace(?1, char(10), '')))
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
