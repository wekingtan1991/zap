//! 剪贴板图片支持：截图类条目入库 / 缩略图 / 回贴。
//! 独立表 clipboard_images（文本表 content UNIQUE 的去重语义不适用于图片），
//! 按 sha256 去重（重复复制同一张图 = 置顶刷新时间）；PNG 全图 + 72px 缩略图
//! 双份存储——列表只拖缩略图（大图按行号点查，与文本 substr 优化同思路）。
//! 编解码用 png crate（图标提取已有依赖），缩略图降采样为手写盒式滤波

use base64::Engine as _;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::sync::atomic::Ordering;

use super::SearchItem;

/// 单张图片入库上限（PNG 字节）：4K 截图通常 <10MB，超限视为异常输入直接放弃
pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
/// 缩略图最长边（像素）：列表行高有限，够辨认即可
const THUMB_MAX: u32 = 72;

pub fn init_table(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS clipboard_images (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             sha256 TEXT NOT NULL UNIQUE,
             width INTEGER NOT NULL,
             height INTEGER NOT NULL,
             size INTEGER NOT NULL,
             data BLOB NOT NULL,
             thumb BLOB NOT NULL,
             created_at INTEGER NOT NULL,
             pinned INTEGER NOT NULL DEFAULT 0
         );",
    );
}

/// RGBA → PNG 编码
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    if rgba.len() != width as usize * height as usize * 4 {
        return Err("RGBA 数据长度与尺寸不符".into());
    }
    let mut buf = Vec::new();
    let mut encoder = png::Encoder::new(&mut buf, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    drop(writer); // 释放对 buf 的借用才能移出
    Ok(buf)
}

/// PNG → (宽, 高, RGBA)。只处理本模块编码的 PNG（恒为 RGBA8）
pub fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    Ok((info.width, info.height, buf[..info.buffer_size()].to_vec()))
}

/// 盒式降采样到最长边 ≤ max（等比，不放大），返回 (宽, 高, RGBA)
fn downscale(rgba: &[u8], width: u32, height: u32, max: u32) -> (u32, u32, Vec<u8>) {
    let scale = (max as f32 / width.max(height) as f32).min(1.0);
    let nw = ((width as f32 * scale).round() as u32).max(1);
    let nh = ((height as f32 * scale).round() as u32).max(1);
    if (nw, nh) == (width, height) {
        return (width, height, rgba.to_vec());
    }
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let sy0 = y as u64 * height as u64 / nh as u64;
        let sy1 = ((y as u64 + 1) * height as u64 / nh as u64).max(sy0 + 1);
        for x in 0..nw {
            let sx0 = x as u64 * width as u64 / nw as u64;
            let sx1 = ((x as u64 + 1) * width as u64 / nw as u64).max(sx0 + 1);
            let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let i = ((sy * width as u64 + sx) * 4) as usize;
                    r += rgba[i] as u64;
                    g += rgba[i + 1] as u64;
                    b += rgba[i + 2] as u64;
                    a += rgba[i + 3] as u64;
                }
            }
            let n = (sy1 - sy0) * (sx1 - sx0);
            let o = ((y * nw + x) * 4) as usize;
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
            out[o + 3] = (a / n) as u8;
        }
    }
    (nw, nh, out)
}

fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// 入库：RGBA 全图编码 PNG + 生成缩略图 + sha256 去重（重复 = 刷新时间置顶）
/// + 保留清理。返回 (行号, 宽, 高)；超限/编码失败返回 None（静默，不入库不打扰）
pub fn store(conn: &Connection, rgba: &[u8], width: u32, height: u32) -> Option<(i64, u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let png = encode_png(rgba, width, height).ok()?;
    if png.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let (tw, th, trgba) = downscale(rgba, width, height, THUMB_MAX);
    let thumb = encode_png(&trgba, tw, th).ok()?;
    let hash: String = Sha256::digest(&png)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let now = now_ms();
    conn.execute(
        "INSERT INTO clipboard_images(sha256, width, height, size, data, thumb, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(sha256) DO UPDATE SET created_at=excluded.created_at",
        params![hash, width, height, png.len() as i64, png, thumb, now],
    )
    .ok()?;
    // 与文本表同规则：保留条数淘汰，固定条目豁免
    let _ = conn.execute(
        "DELETE FROM clipboard_images WHERE pinned=0 AND id NOT IN (
             SELECT id FROM clipboard_images ORDER BY created_at DESC LIMIT ?1
         )",
        params![super::clipboard::RETENTION_LIMIT.load(Ordering::Relaxed)],
    );
    let rowid: i64 = conn
        .query_row(
            "SELECT id FROM clipboard_images WHERE sha256=?1",
            params![hash],
            |r| r.get(0),
        )
        .ok()?;
    Some((rowid, width, height))
}

/// 图片条目：id = "clipimg:{rowid}"，icon = 缩略图 data URL（列表直接渲染）。
/// pinned 标记沿用 📌 前缀约定（VaultRowPin 与 isPinned 共用）
#[allow(clippy::too_many_arguments)]
fn build_item(
    rowid: i64,
    w: u32,
    h: u32,
    size: i64,
    thumb: Vec<u8>,
    created_at: i64,
    now: i64,
    pinned: bool,
) -> SearchItem {
    let title = if pinned {
        format!("📌 图片 {w}×{h}")
    } else {
        format!("图片 {w}×{h}")
    };
    SearchItem {
        id: format!("clipimg:{rowid}"),
        title,
        subtitle: format!(
            "剪贴板图片 · {} · {}，Enter 回贴",
            human_size(size as usize),
            super::clipboard::ago(created_at, now)
        ),
        kind: "clipimg".into(),
        // target = 行号：回贴按行号点查全图
        target: rowid.to_string(),
        icon: Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(thumb)
        )),
        score: 0.0,
    }
}

/// 空查询（面板呼出）时的图片条目：固定置顶 + 时间倒序，
/// 返回 (pinned, created_at, item) 供调用方与文本条目按同键合并排序
pub fn search_recent(conn: &Connection, limit: usize) -> Vec<(i64, i64, SearchItem)> {
    let now = now_ms();
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, width, height, size, thumb, created_at, pinned FROM clipboard_images
         ORDER BY pinned DESC, created_at DESC LIMIT ?1",
    ) else {
        return Vec::new();
    };
    let rows = stmt
        .query_map(params![limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Vec<u8>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .into_iter()
        .flatten()
        .flatten()
        .collect::<Vec<_>>();
    rows.into_iter()
        .map(|(id, w, h, size, thumb, t, pinned)| {
            (
                pinned,
                t,
                build_item(id, w as u32, h as u32, size, thumb, t, now, pinned == 1),
            )
        })
        .collect()
}

/// 按行号取全图（回贴用）：历史保留期内行号恒有效
pub fn get(conn: &Connection, id: i64) -> Option<(u32, u32, Vec<u8>)> {
    conn.query_row(
        "SELECT width, height, data FROM clipboard_images WHERE id=?1",
        params![id],
        |r| {
            Ok((
                r.get::<_, i64>(0)? as u32,
                r.get::<_, i64>(1)? as u32,
                r.get::<_, Vec<u8>>(2)?,
            ))
        },
    )
    .ok()
}

/// 行号是否仍有图片：open_item 的同步错误路径用，只探存在性不拖 PNG 大字段
pub fn exists(conn: &Connection, id: i64) -> bool {
    conn.query_row(
        "SELECT 1 FROM clipboard_images WHERE id=?1",
        params![id],
        |_| Ok(()),
    )
    .is_ok()
}

/// 图片表固定/取消固定（pin_toggle 在文本表未命中时落到这里，返回操作后状态）
pub fn pin_toggle(conn: &Connection, id: i64) -> Result<bool, String> {
    let changed = conn
        .execute(
            "UPDATE clipboard_images SET pinned = 1 - pinned WHERE id=?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("条目不存在".into());
    }
    let pinned: i64 = conn
        .query_row(
            "SELECT pinned FROM clipboard_images WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(pinned == 1)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn mem_conn() -> Connection {
        super::super::clipboard::open_db(Path::new(":memory:")).expect("mem db")
    }

    /// 确定性伪随机 RGBA（同尺寸同内容，用于去重断言）
    fn sample(w: u32, h: u32) -> Vec<u8> {
        (0..w as usize * h as usize * 4)
            .map(|i| (i % 251) as u8)
            .collect()
    }

    #[test]
    fn png编解码往返() {
        let src = sample(4, 2);
        let png = encode_png(&src, 4, 2).unwrap();
        let (w, h, rgba) = decode_png(&png).unwrap();
        assert_eq!((w, h), (4, 2));
        assert_eq!(rgba, src);
    }

    #[test]
    fn 入库去重与尺寸() {
        let conn = mem_conn();
        let (id1, w, h) = store(&conn, &sample(4, 2), 4, 2).expect("入库");
        assert_eq!((w, h), (4, 2));
        // 同图再入库：sha256 去重为同一行（时间刷新）
        let (id2, _, _) = store(&conn, &sample(4, 2), 4, 2).expect("重复入库");
        assert_eq!(id1, id2, "同图应去重为同一行");
        // 不同图另起一行
        let mut other = sample(4, 2);
        other[0] = 255;
        let (id3, _, _) = store(&conn, &other, 4, 2).expect("不同图入库");
        assert_ne!(id1, id3);
        let items = search_recent(&conn, 10);
        assert_eq!(items.len(), 2);
        let first = &items[0].2;
        assert!(first.id.starts_with("clipimg:"));
        assert!(
            first
                .icon
                .as_deref()
                .unwrap_or("")
                .starts_with("data:image/png"),
            "icon 应为缩略图 data URL"
        );
        assert!(first.title.contains('×'), "标题应含尺寸");
        // 尺寸为 0 拒绝
        assert!(store(&conn, &[], 0, 0).is_none());
    }

    #[test]
    fn 缩略图降采样() {
        let src = sample(200, 100);
        let (nw, nh, out) = downscale(&src, 200, 100, THUMB_MAX);
        assert_eq!((nw, nh), (72, 36), "等比缩到最长边 72");
        assert_eq!(out.len(), (nw * nh * 4) as usize);
        // 不放大：小图原样返回
        let small = sample(8, 8);
        let (w2, h2, out2) = downscale(&small, 8, 8, THUMB_MAX);
        assert_eq!((w2, h2), (8, 8));
        assert_eq!(out2.len(), small.len());
    }

    #[test]
    fn pin_toggle文本表未中时落图片表() {
        let conn = mem_conn();
        let (img_id, _, _) = store(&conn, &sample(4, 2), 4, 2).expect("入库");
        assert!(super::super::clipboard::pin_toggle(&conn, img_id).unwrap());
        let pinned: i64 = conn
            .query_row(
                "SELECT pinned FROM clipboard_images WHERE id=?1",
                params![img_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pinned, 1);
    }

    #[test]
    fn 保留清理与固定豁免() {
        let conn = mem_conn();
        let (keep, _, _) = store(&conn, &sample(4, 2), 4, 2).expect("入库");
        super::super::clipboard::pin_toggle(&conn, keep).unwrap();
        let mut other = sample(4, 2);
        other[0] = 1;
        store(&conn, &other, 4, 2).expect("入库");
        let mut third = sample(4, 2);
        third[0] = 2;
        store(&conn, &third, 4, 2).expect("入库");
        // 只留 1 条：固定者存活
        conn.execute(
            "DELETE FROM clipboard_images WHERE pinned=0 AND id NOT IN (
                 SELECT id FROM clipboard_images ORDER BY created_at DESC LIMIT 1
             )",
            [],
        )
        .unwrap();
        assert!(get(&conn, keep).is_some(), "固定图片不被清理");
    }
}
