use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use super::SearchItem;

/// 本地应用索引：开始菜单 .lnk（UWP 留待后续版本），提取应用图标。
/// 面板每次呼出时后台 refresh() 重建，新装/卸载的应用无需重启即可生效。
pub struct AppIndex {
    roots: Vec<PathBuf>,
    cache: Option<PathBuf>,
    /// 是否收录 UWP/商店应用（测试关闭，避免依赖本机 AppsFolder）
    uwp: bool,
    /// 防止重扫并发叠加：上次未完成时跳过本次
    refreshing: AtomicBool,
    entries: RwLock<Arc<Vec<AppEntry>>>,
}

struct AppEntry {
    name: String,
    path: String,
    /// 应用图标 PNG data URL；提取失败为 None，前端退回文字角标
    icon: Option<String>,
}

impl AppIndex {
    /// cache_dir 缓存提取出的图标（键 = lnk 路径+mtime 哈希），传 None 则只提内存不落盘
    pub fn scan(cache_dir: Option<&Path>) -> Self {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(p) = std::env::var("APPDATA") {
            roots.push(PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"));
        }
        if let Ok(p) = std::env::var("PROGRAMDATA") {
            roots.push(PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"));
        }
        Self::new(roots, cache_dir.map(Path::to_path_buf), true)
    }

    fn new(roots: Vec<PathBuf>, cache: Option<PathBuf>, uwp: bool) -> Self {
        let entries = Self::collect(&roots, cache.as_deref(), uwp);
        Self {
            roots,
            cache,
            uwp,
            refreshing: AtomicBool::new(false),
            entries: RwLock::new(Arc::new(entries)),
        }
    }

    fn collect(roots: &[PathBuf], cache: Option<&Path>, uwp: bool) -> Vec<AppEntry> {
        if let Some(dir) = cache {
            let _ = fs::create_dir_all(dir);
        }
        let mut entries = Vec::new();
        for root in roots {
            walk(root, 0, cache, &mut entries);
        }
        if uwp {
            scan_uwp(cache, &mut entries);
        }
        entries
    }

    /// 后台重建索引（含图标缓存命中）。返回 false = 上一次重扫尚未完成，本次跳过
    pub fn refresh(&self) -> bool {
        if self.refreshing.swap(true, Ordering::SeqCst) {
            return false;
        }
        let fresh = Self::collect(&self.roots, self.cache.as_deref(), self.uwp);
        if let Some(dir) = &self.cache {
            Self::prune_cache(dir, &fresh);
        }
        *self.entries.write().expect("app index poisoned") = Arc::new(fresh);
        self.refreshing.store(false, Ordering::SeqCst);
        true
    }

    /// 清掉已卸载应用遗留的图标缓存文件
    fn prune_cache(dir: &Path, entries: &[AppEntry]) {
        let keep: HashSet<String> = entries
            .iter()
            .map(|e| format!("{:016x}.png", cache_key(&e.path)))
            .collect();
        let Ok(rd) = fs::read_dir(dir) else { return };
        for f in rd.flatten() {
            let fname = f.file_name();
            let Some(name) = fname.to_str() else { continue };
            if name.ends_with(".png") && !keep.contains(name) {
                let _ = fs::remove_file(f.path());
            }
        }
    }

    /// 测试/观测用：全部条目数与其中带图标的条目数
    #[cfg(test)]
    pub(crate) fn stats(&self) -> (usize, usize) {
        let es = self.entries.read().expect("app index poisoned");
        (es.len(), es.iter().filter(|e| e.icon.is_some()).count())
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchItem> {
        let entries = self.entries.read().expect("app index poisoned").clone();
        let mut scored: Vec<(f32, usize)> = Vec::new();

        for (i, e) in entries.iter().enumerate() {
            if let Some(s) = super::score_match(&e.name, query) {
                // 名字越短权重略高
                scored.push((s - e.name.chars().count() as f32 * 0.05, i));
            }
        }

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        scored
            .into_iter()
            .map(|(score, i)| {
                let e = &entries[i];
                SearchItem {
                    id: format!("app:{i}"),
                    title: e.name.clone(),
                    subtitle: e.path.clone(),
                    kind: "app".into(),
                    target: e.path.clone(),
                    icon: e.icon.clone(),
                    score,
                }
            })
            .collect()
    }
}

/// 卸载器快捷方式（"卸载XX" / "Uninstall…"）：误触即启动卸载程序，不作应用收录
fn is_uninstaller(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("卸载") || n.contains("uninst")
}

fn walk(dir: &Path, depth: u8, cache: Option<&Path>, out: &mut Vec<AppEntry>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk(&p, depth + 1, cache, out);
        } else if p.extension().is_some_and(|ext| ext == "lnk") {
            let Some(stem) = p.file_stem() else { continue };
            let name = stem.to_string_lossy().to_string();
            if is_uninstaller(&name) {
                continue;
            }
            let path = p.to_string_lossy().to_string();
            let icon = icon_data_url(cache, &path);
            out.push(AppEntry { name, path, icon });
        }
    }
}

/// 图标 data URL：优先读磁盘缓存，未命中则提取并写回（写失败只影响下次启动速度）
fn icon_data_url(cache: Option<&Path>, lnk: &str) -> Option<String> {
    match cache {
        Some(dir) => {
            let file = dir.join(format!("{:016x}.png", cache_key(lnk)));
            if let Ok(bytes) = fs::read(&file) {
                return Some(to_data_url(&bytes));
            }
            let png = extract_icon_png(lnk)?;
            let _ = fs::write(&file, &png);
            Some(to_data_url(&png))
        }
        None => extract_icon_png(lnk).map(|png| to_data_url(&png)),
    }
}

/// FNV-1a(路径 + lnk mtime)：mtime 入键，应用更新后图标缓存自动失效
fn cache_key(lnk: &str) -> u64 {
    fn feed(h: &mut u64, bytes: &[u8]) {
        for &b in bytes {
            *h ^= b as u64;
            *h = h.wrapping_mul(0x100000001b3);
        }
    }
    let mut h: u64 = 0xcbf29ce484222325;
    feed(&mut h, lnk.as_bytes());
    if let Ok(d) = fs::metadata(lnk)
        .and_then(|md| md.modified())
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default())
    {
        feed(&mut h, &d.as_secs().to_le_bytes());
        feed(&mut h, &d.subsec_nanos().to_le_bytes());
    }
    h
}

fn to_data_url(png: &[u8]) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    format!("data:image/png;base64,{}", B64.encode(png))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UWP/商店应用：枚举 shell:AppsFolder，补齐开始菜单 .lnk 覆盖不到的部分
/// （如 Windows 计算器）。target 存 `shell:AppsFolder\<AUMID>`，打开时经 explorer。
fn scan_uwp(cache: Option<&Path>, out: &mut Vec<AppEntry>) {
    use std::iter::once;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, IBindCtx,
    };
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, IEnumShellItems, IShellItem, IShellItemImageFactory,
        SHCreateItemFromParsingName, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
        SIIGBF_BIGGERSIZEOK,
    };
    use windows::core::PCWSTR;

    /// GetDisplayName 返回的 PWSTR → String 并释放 COM 内存
    fn pw_take(pw: windows::core::PWSTR) -> String {
        unsafe {
            let mut len = 0usize;
            while *pw.as_ptr().add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(pw.as_ptr(), len));
            CoTaskMemFree(Some(pw.as_ptr().cast()));
            s
        }
    }

    unsafe {
        // COM 按线程初始化；线程已是其他模式时忽略错误继续用现有初始化
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let path: Vec<u16> = "shell:AppsFolder".encode_utf16().chain(once(0)).collect();
        let Some(folder) = SHCreateItemFromParsingName::<_, Option<&IBindCtx>, IShellItem>(
            PCWSTR(path.as_ptr()),
            None,
        )
        .ok() else {
            eprintln!("zap: shell:AppsFolder 打开失败，UWP 应用未收录");
            return;
        };
        let Some(items) = folder
            .BindToHandler::<Option<&IBindCtx>, IEnumShellItems>(None, &BHID_EnumItems)
            .ok()
        else {
            eprintln!("zap: shell:AppsFolder 枚举失败");
            return;
        };

        let mut seen: HashSet<String> = out.iter().map(|e| e.name.to_lowercase()).collect();
        let mut arr: [Option<IShellItem>; 8] = std::array::from_fn(|_| None);
        loop {
            let mut fetched = 0u32;
            let _ = items.Next(&mut arr, Some(&mut fetched));
            if fetched == 0 {
                break;
            }
            for it in arr.iter_mut().take(fetched as usize) {
                let Some(it) = it.take() else { continue };
                let Ok(title) = it.GetDisplayName(SIGDN_NORMALDISPLAY) else {
                    continue;
                };
                let title = pw_take(title);
                let Ok(aumid) = it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) else {
                    continue;
                };
                let aumid = pw_take(aumid);
                // 只收 UWP/商店包；桌面程序已由 .lnk 覆盖，按名去重
                if !aumid.contains('!') || is_uninstaller(&title) {
                    continue;
                }
                if !seen.insert(title.to_lowercase()) {
                    continue;
                }
                let target = format!("shell:AppsFolder\\{aumid}");
                let icon = (|| -> Option<String> {
                    if let Some(dir) = cache {
                        let file = dir.join(format!("{:016x}.png", cache_key(&target)));
                        if let Ok(bytes) = fs::read(&file) {
                            return Some(to_data_url(&bytes));
                        }
                    }
                    let png = (|| -> Option<Vec<u8>> {
                        let app_path: Vec<u16> =
                            target.encode_utf16().chain(std::iter::once(0)).collect();
                        // 直接向条目请求图像工厂接口
                        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(
                            PCWSTR(app_path.as_ptr()),
                            None::<&IBindCtx>,
                        )
                        .ok()?;
                        let hbm = factory
                            .GetImage(SIZE { cx: 64, cy: 64 }, SIIGBF_BIGGERSIZEOK)
                            .ok()?;
                        let png =
                            bitmap_to_rgba(hbm).and_then(|(w, h, buf)| bitmap_to_png(w, h, buf));
                        let _ = DeleteObject(hbm.into());
                        png
                    })()?;
                    if let Some(dir) = cache {
                        let file = dir.join(format!("{:016x}.png", cache_key(&target)));
                        let _ = fs::write(&file, &png);
                    }
                    Some(to_data_url(&png))
                })();
                out.push(AppEntry {
                    name: title,
                    path: target,
                    icon,
                });
            }
        }
    }
}

/// 从 .lnk 提取目标图标为 PNG（系统 32x32 大图标）；失败返回 None，前端退回角标
fn extract_icon_png(lnk: &str) -> Option<Vec<u8>> {
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
    use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;
    use windows::core::PCWSTR;

    let wide: Vec<u16> = wide(lnk);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 卸载器名称识别() {
        assert!(is_uninstaller("卸载豆包"));
        assert!(is_uninstaller("卸载 微信开发者工具"));
        assert!(is_uninstaller("Uninstall"));
        assert!(is_uninstaller("Uninstall Mem Reduct"));
        assert!(!is_uninstaller("豆包"));
        assert!(!is_uninstaller("Mem Reduct"));
    }

    #[test]
    fn 扫描时跳过卸载快捷方式() {
        let dir = std::env::temp_dir().join("zap_app_index_walk_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("豆包.lnk"), "").unwrap();
        fs::write(dir.join("卸载豆包.lnk"), "").unwrap();
        fs::write(dir.join("Uninstall.lnk"), "").unwrap();

        let mut out = Vec::new();
        walk(&dir, 0, None, &mut out);
        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["豆包"]);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 缓存键稳定可复现() {
        let dir = std::env::temp_dir().join("zap_app_index_key_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.lnk");
        fs::write(&f, b"x").unwrap();
        let p = f.to_string_lossy().to_string();
        assert_eq!(cache_key(&p), cache_key(&p), "同路径同 mtime 应得同键");
        // 假 lnk（无文件）也能算出键而不 panic
        let _ = cache_key("Z:\\不存在\\bogus.lnk");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// 本机真实开始菜单的图标提取产出观察（CI/无图标环境不适用，故 ignore）
    #[test]
    #[ignore = "机器相关冒烟：cargo test --lib -- --ignored --nocapture"]
    fn 真实开始菜单图标提取冒烟() {
        let cache = std::env::temp_dir().join("zap_icon_cache_smoke");
        let _ = fs::remove_dir_all(&cache);
        let t0 = std::time::Instant::now();
        let idx = AppIndex::scan(Some(&cache));
        let (total, with_icon) = idx.stats();
        let cold = t0.elapsed();
        let t1 = std::time::Instant::now();
        let idx2 = AppIndex::scan(Some(&cache));
        let warm = t1.elapsed();
        let cached = fs::read_dir(&cache).map(|rd| rd.count()).unwrap_or(0);
        println!(
            "图标提取 {with_icon}/{total} 条 · 冷提取 {cold:?} · 缓存重扫 {warm:?} · 缓存文件 {cached} 个"
        );
        assert!(total > 0, "开始菜单为空？");
        assert_eq!(total, idx2.stats().0);
        let _ = fs::remove_dir_all(&cache);
    }

    #[test]
    fn 重扫反映安装与卸载() {
        let dir = std::env::temp_dir().join("zap_app_index_rescan_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("豆包.lnk"), "").unwrap();

        let idx = AppIndex::new(vec![dir.clone()], None, false);
        assert_eq!(idx.search("豆包", 5).len(), 1);

        // 模拟新装应用：重扫前搜不到，重扫后可见
        fs::write(dir.join("记事本Plus.lnk"), "").unwrap();
        assert!(idx.search("记事本", 5).is_empty());
        assert!(idx.refresh());
        assert_eq!(idx.search("记事本", 5).len(), 1);

        // 模拟卸载：重扫后消失
        fs::remove_file(dir.join("豆包.lnk")).unwrap();
        assert!(idx.refresh());
        assert!(idx.search("豆包", 5).is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 重扫清理孤儿图标缓存() {
        let dir = std::env::temp_dir().join("zap_app_index_prune_test");
        let cache = std::env::temp_dir().join("zap_app_index_prune_cache");
        let _ = (fs::remove_dir_all(&dir), fs::remove_dir_all(&cache));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("豆包.lnk"), "").unwrap();

        let idx = AppIndex::new(vec![dir.clone()], Some(cache.clone()), false);
        assert_eq!(idx.stats().0, 1);

        // 造一个不属于任何应用的孤儿缓存文件
        let orphan = cache.join("deadbeef.png");
        fs::write(&orphan, b"junk").unwrap();

        // 卸载后重扫：目录为空 → 所有缓存文件都是孤儿
        fs::remove_file(dir.join("豆包.lnk")).unwrap();
        assert!(idx.refresh());
        assert!(!orphan.exists(), "孤儿缓存应被清理");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&cache);
    }
}
