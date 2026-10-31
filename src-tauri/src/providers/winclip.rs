//! Win32/arboard 剪贴板写入层（自 clipboard.rs 拆出，审计 D2）：
//! zap 自身复制抑制窗口、open+set 整操作退避（retrying）、敏感内容专用
//! 通道（copy_text_private，排除 Win+V 与云剪贴板）、CF_DIBV5 图片直写
//! （绕开 arboard set_image 的 PNG 编码秒级延迟）。读侧监听与历史存储
//! 留在 clipboard.rs

use std::sync::atomic::{AtomicU64, Ordering};

use super::util::now_ms;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
