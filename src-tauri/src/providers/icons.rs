//! 图标提取与编码（自 apps.rs 拆出，审计 D5）：UWP COM 提取 / SHGetFileInfoW
//! 通用路径提取 / HBITMAP·HICON → RGBA → PNG 编码 / data URL 包装。
//! 消费方：apps 索引扫描、stage 列表图标、剪贴板图片（to_data_url 语义同源）

use super::util::wide;

pub(crate) fn to_data_url(png: &[u8]) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    format!("data:image/png;base64,{}", B64.encode(png))
}

/// UWP/商店应用：枚举 shell:AppsFolder，补齐开始菜单 .lnk 覆盖不到的部分
/// （如 Windows 计算器）。target 存 `shell:AppsFolder\<AUMID>`，打开时经 explorer。
/// UWP（shell:AppsFolder\<AUMID>）条目图标：向条目请求图像工厂接口提取。
/// 供 spawn_icon_backfill 在后台线程调用（scan_uwp 首扫不再现场提取）
pub(crate) fn extract_uwp_icon(target: &str) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, IBindCtx};
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
    };
    use windows::core::PCWSTR;

    let app_path = wide(target);
    unsafe {
        // COM 按线程初始化；已是其他模式时忽略错误继续用现有初始化
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(app_path.as_ptr()), None::<&IBindCtx>).ok()?;
        let hbm = factory
            .GetImage(SIZE { cx: 64, cy: 64 }, SIIGBF_BIGGERSIZEOK)
            .ok()?;
        let png = bitmap_to_rgba(hbm).and_then(|(w, h, buf)| bitmap_to_png(w, h, buf));
        let _ = DeleteObject(hbm.into());
        png
    }
}

/// 从路径提取图标为 PNG（SHGetFileInfoW 系统 32x32 大图标，对任意文件/目录路径
/// 通用，不限于 .lnk）；失败返回 None，前端退回角标
pub(crate) fn extract_icon_png(path: &str) -> Option<Vec<u8>> {
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;
    use windows::core::PCWSTR;

    let wide: Vec<u16> = wide(path);
    unsafe {
        let mut info: SHFILEINFOW = std::mem::zeroed();
        let ok = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if ok == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let png = hicon_to_png(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        png
    }
}

/// HBITMAP → 32 位 RGBA 像素（自上而下）
fn bitmap_to_rgba(hbm: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        GetDC, GetDIBits, GetObjectW, ReleaseDC,
    };

    unsafe {
        let hdc = GetDC(None);
        let mem = CreateCompatibleDC(Some(hdc));
        let mut bm = BITMAP::default();
        let got = GetObjectW(
            hbm.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        let mut result = None;
        if got != 0 && w > 0 && h > 0 {
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h; // 负高 = 自上而下的行序
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;
            let mut buf = vec![0u8; w as usize * h as usize * 4];
            let lines = GetDIBits(
                mem,
                hbm,
                0,
                h as u32,
                Some(buf.as_mut_ptr().cast()),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            if lines == h {
                for px in buf.as_chunks_mut::<4>().0 {
                    px.swap(0, 2); // BGRA → RGBA
                }
                if buf.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
                    // 无 alpha 通道的旧式图标按不透明处理
                    for px in buf.as_chunks_mut::<4>().0 {
                        px[3] = 255;
                    }
                }
                result = Some((w as u32, h as u32, buf));
            }
        }
        let _ = DeleteDC(mem);
        let _ = ReleaseDC(None, hdc);
        result
    }
}

/// 位图 → PNG
fn bitmap_to_png(w: u32, h: u32, buf: Vec<u8>) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(&buf).ok()?;
    }
    Some(out)
}

/// HICON → RGBA
fn hicon_to_rgba(
    hicon: windows::Win32::UI::WindowsAndMessaging::HICON,
) -> Option<(u32, u32, Vec<u8>)> {
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, ICONINFO};

    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(hicon, &mut ii).ok()?;
        let result = if !ii.hbmColor.is_invalid() {
            bitmap_to_rgba(ii.hbmColor)
        } else {
            None
        };
        let _ = DeleteObject(ii.hbmColor.into());
        let _ = DeleteObject(ii.hbmMask.into());
        result
    }
}

/// HICON → PNG
fn hicon_to_png(hicon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    let (w, h, buf) = hicon_to_rgba(hicon)?;
    bitmap_to_png(w, h, buf)
}
