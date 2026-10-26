use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use rusqlite::{Connection, params};

use super::SearchItem;

/// 旧索引条目的图标按缓存键转内存复用：重扫免掉几百次 PNG 读盘 + base64
type MemIcons = HashMap<u64, String>;

/// 面板呼出触发的重扫节流：间隔内跳过，避免每次呼出都全量走目录树 + UWP 枚举
pub(crate) const REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(10);

/// 本地应用索引：开始菜单 .lnk（UWP 留待后续版本），提取应用图标。
/// 面板呼出时后台 refresh_if_stale() 节流重建，新装/卸载的应用即可搜到。
pub struct AppIndex {
    roots: Vec<PathBuf>,
    cache: Option<PathBuf>,
    /// 是否收录 UWP/商店应用（测试关闭，避免依赖本机 AppsFolder）
    uwp: bool,
    /// 防止重扫并发叠加：上次未完成时跳过本次
    refreshing: AtomicBool,
    /// 上次完成重扫的时刻（构造时的首扫计入），节流判断用
    last_refresh: Mutex<Option<Instant>>,
    entries: RwLock<Arc<Vec<AppEntry>>>,
}

struct AppEntry {
    name: String,
    path: String,
    /// 应用图标 PNG data URL；提取失败为 None，前端退回文字角标
    icon: Option<String>,
    /// 图标缓存键（路径+mtime 哈希）；内存复用与缓存清理共用，省去重算的 stat
    icon_key: u64,
    /// 名称的匹配形态（小写/拼音全拼/首字母），扫描时算一次
    forms: super::MatchForms,
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
        let entries = Self::collect(&roots, cache.as_deref(), uwp, &MemIcons::new());
        Self {
            roots,
            cache,
            uwp,
            refreshing: AtomicBool::new(false),
            last_refresh: Mutex::new(Some(Instant::now())),
            entries: RwLock::new(Arc::new(entries)),
        }
    }

    fn collect(
        roots: &[PathBuf],
        cache: Option<&Path>,
        uwp: bool,
        mem: &MemIcons,
    ) -> Vec<AppEntry> {
        if let Some(dir) = cache {
            let _ = fs::create_dir_all(dir);
        }
        let mut entries = Vec::new();
        for root in roots {
            walk(root, 0, cache, mem, &mut entries);
        }
        if uwp {
            scan_uwp(cache, mem, &mut entries);
        }
        entries
    }

    /// 后台重建索引（含图标缓存命中）。返回 false = 上一次重扫尚未完成，本次跳过
    pub fn refresh(&self) -> bool {
        if self.refreshing.swap(true, Ordering::SeqCst) {
            return false;
        }
        let mem: MemIcons = self
            .entries
            .read()
            .expect("app index poisoned")
            .iter()
            .filter_map(|e| e.icon.clone().map(|ic| (e.icon_key, ic)))
            .collect();
        let fresh = Self::collect(&self.roots, self.cache.as_deref(), self.uwp, &mem);
        if let Some(dir) = &self.cache {
            Self::prune_cache(dir, &fresh);
        }
        *self.entries.write().expect("app index poisoned") = Arc::new(fresh);
        self.refreshing.store(false, Ordering::SeqCst);
        *self.last_refresh.lock().expect("last_refresh poisoned") = Some(Instant::now());
        true
    }

    /// 面板呼出用的节流版重扫：距上次完成不足 min_interval 则跳过（返回 false）
    pub fn refresh_if_stale(&self, min_interval: Duration) -> bool {
        {
            let last = self.last_refresh.lock().expect("last_refresh poisoned");
            if last.is_some_and(|t| t.elapsed() < min_interval) {
                return false;
            }
        }
        self.refresh()
    }

    /// 清掉已卸载应用遗留的图标缓存文件
    fn prune_cache(dir: &Path, entries: &[AppEntry]) {
        let keep: HashSet<String> = entries
            .iter()
            .map(|e| format!("{:016x}.png", e.icon_key))
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

    pub fn search(
        &self,
        query: &str,
        limit: usize,
        launches: &HashMap<String, i64>,
    ) -> Vec<SearchItem> {
        let entries = self.entries.read().expect("app index poisoned").clone();
        let q = query.trim().to_lowercase();
        let mut scored: Vec<(f32, usize)> = Vec::new();

        for (i, e) in entries.iter().enumerate() {
            if let Some(s) = e.forms.score(&q) {
                // 名字越短权重略高；启动次数加权（同档内常用者靠前，+0.5×次数
                // 上限 15 分，跨得过同档微差、越不过前缀>包含的 20 分档差）
                let used = launches.get(&e.path).copied().unwrap_or(0);
                let boost = (used.min(30) as f32) * 0.5;
                scored.push((s + boost - e.name.chars().count() as f32 * 0.05, i));
            }
        }

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        scored
            .into_iter()
            .map(|(score, i)| {
                let e = &entries[i];
                SearchItem {
                    // id 由路径哈希派生：重扫/重启后同一条目 id 不变（固定/启动计数跨会话寻址）
                    id: format!("app:{:016x}", super::stable_key(&e.path)),
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

// ---------- 启动统计（frecency）----------

pub fn init_stats(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_stats (
             target TEXT PRIMARY KEY,
             launches INTEGER NOT NULL DEFAULT 0,
             last_at INTEGER NOT NULL DEFAULT 0
         );",
    );
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// open_item 记一次启动：按 target（lnk 路径 / shell:AppsFolder AUMID）计数
pub fn record_launch(conn: &Connection, target: &str) {
    let _ = conn.execute(
        "INSERT INTO app_stats(target, launches, last_at) VALUES(?1, 1, ?2)
         ON CONFLICT(target) DO UPDATE SET launches=launches+1, last_at=excluded.last_at",
        params![target, now_ms()],
    );
}

/// 全量加载启动计数（表里只有真实启动过的应用，通常几十行）
fn load_launch_counts(conn: &Connection) -> HashMap<String, i64> {
    let Ok(mut stmt) = conn.prepare("SELECT target, launches FROM app_stats") else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) else {
        return HashMap::new();
    };
    rows.flatten().collect()
}

/// 卸载器快捷方式（"卸载XX" / "Uninstall…"）：误触即启动卸载程序，不作应用收录
fn is_uninstaller(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("卸载") || n.contains("uninst")
}

fn walk(dir: &Path, depth: u8, cache: Option<&Path>, mem: &MemIcons, out: &mut Vec<AppEntry>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk(&p, depth + 1, cache, mem, out);
        } else if p.extension().is_some_and(|ext| ext == "lnk") {
            let Some(stem) = p.file_stem() else { continue };
            let name = stem.to_string_lossy().to_string();
            if is_uninstaller(&name) {
                continue;
            }
            let path = p.to_string_lossy().to_string();
            let icon_key = cache_key(&path);
            let icon = icon_data_url(cache, &path, icon_key, mem);
            let forms = super::MatchForms::new(&name);
            out.push(AppEntry {
                name,
                path,
                icon,
                icon_key,
                forms,
            });
        }
    }
}

/// 图标 data URL：内存复用 → 磁盘缓存 → 现场提取（写失败只影响下次启动速度）
fn icon_data_url(cache: Option<&Path>, lnk: &str, key: u64, mem: &MemIcons) -> Option<String> {
    if let Some(hit) = mem.get(&key) {
        return Some(hit.clone());
    }
    match cache {
        Some(dir) => {
            let file = dir.join(format!("{:016x}.png", key));
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
    let mut h = super::stable_key(lnk);
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
fn scan_uwp(cache: Option<&Path>, mem: &MemIcons, out: &mut Vec<AppEntry>) {
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
                let icon_key = cache_key(&target);
                let icon = if let Some(hit) = mem.get(&icon_key) {
                    Some(hit.clone())
                } else {
                    (|| -> Option<String> {
                        if let Some(dir) = cache {
                            let file = dir.join(format!("{:016x}.png", icon_key));
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
                            let png = bitmap_to_rgba(hbm)
                                .and_then(|(w, h, buf)| bitmap_to_png(w, h, buf));
                            let _ = DeleteObject(hbm.into());
                            png
                        })()?;
                        if let Some(dir) = cache {
                            let file = dir.join(format!("{:016x}.png", icon_key));
                            let _ = fs::write(&file, &png);
                        }
                        Some(to_data_url(&png))
                    })()
                };
                let forms = super::MatchForms::new(&title);
                out.push(AppEntry {
                    name: title,
                    path: target,
                    icon,
                    icon_key,
                    forms,
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

/// 展开路径中的环境变量（%ProgramFiles% 等）。展开失败原样返回
fn expand_env_vars(path: &str) -> String {
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows::core::PCWSTR;

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        // 先探所需长度（含结尾 NUL），再取展开结果
        let need = ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), None);
        if need == 0 {
            return path.to_string();
        }
        let mut buf = vec![0u16; need as usize];
        let written = ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), Some(&mut buf));
        if written == 0 || written as usize > buf.len() {
            return path.to_string();
        }
        String::from_utf16_lossy(&buf[..(written as usize - 1).min(buf.len() - 1)])
    }
}

/// 解析 .lnk 指向的真实目标路径（IShellLinkW + IPersistFile）。
/// 非 lnk 或解析失败返回 None，调用方按原路径处理。
/// 次级操作（管理员运行/所在文件夹/复制路径）要的是真实 exe，不是开始菜单里的快捷方式。
/// 注意：SLGP_RAWPATH 按设计不展开环境变量，且可能是相对路径——两者都必须处理，
/// 否则管理员运行/定位拿到坏路径必失败（复制路径不校验存在性，坏路径会静默扩散）
pub fn resolve_lnk_target(path: &str) -> Option<String> {
    // 非 .lnk 直接退出：COM 初始化 + IPersistFile::Load 对普通文件/URL 必然失败，
    // 白走一趟（调用方对 None 回退原始路径，语义不变）
    if !path.to_lowercase().ends_with(".lnk") {
        return None;
    }
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        IPersistFile, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, SLGP_RAWPATH, ShellLink};
    use windows::core::{Interface, PCWSTR};

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        // COM 按线程初始化；已是其他模式时忽略错误继续用现有初始化
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = link.cast().ok()?;
        pf.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        // SLGP_RAWPATH = 4（windows 0.62 的 GetPath 收裸指针 + u32）
        link.GetPath(&mut buf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
            .ok()?;
        let end = buf.iter().position(|&c| c == 0)?;
        let raw = String::from_utf16_lossy(&buf[..end]);
        if raw.is_empty() {
            return None;
        }
        // 环境变量展开；相对路径拼 lnk 的工作目录（lnk 自身目录兜底）
        let mut target = expand_env_vars(&raw);
        if !std::path::Path::new(&target).is_absolute() {
            let mut wd_buf = [0u16; 1024];
            let work_dir = link
                .GetWorkingDirectory(&mut wd_buf)
                .ok()
                .and_then(|_| {
                    let e = wd_buf.iter().position(|&c| c == 0)?;
                    let wd = String::from_utf16_lossy(&wd_buf[..e]);
                    (!wd.is_empty()).then_some(wd)
                })
                .unwrap_or_else(|| {
                    std::path::Path::new(path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default()
                });
            let base = expand_env_vars(&work_dir);
            target = std::path::Path::new(&base)
                .join(&target)
                .to_string_lossy()
                .to_string();
        }
        (!target.is_empty()).then_some(target)
    }
}

/// 应用源插件适配。引擎另持 `Arc<AppsPlugin>` 具体句柄：
/// 呼出时的 refresh_if_stale 要走具体类型，不经过 trait 对象
pub struct AppsPlugin {
    pub index: AppIndex,
}

impl super::plugin::Plugin for AppsPlugin {
    /// 启动统计的读取走 clip 连接（表小、全量快），needs_conn = true
    fn needs_conn(&self) -> bool {
        true
    }
    fn search(&self, query: &str, conn: Option<&Connection>) -> Vec<SearchItem> {
        let launches = conn.map(load_launch_counts).unwrap_or_default();
        self.index.search(query, 8, &launches)
    }
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
        walk(&dir, 0, None, &MemIcons::new(), &mut out);
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
        assert_eq!(idx.search("豆包", 5, &HashMap::new()).len(), 1);

        // 模拟新装应用：重扫前搜不到，重扫后可见
        fs::write(dir.join("记事本Plus.lnk"), "").unwrap();
        assert!(idx.search("记事本", 5, &HashMap::new()).is_empty());
        assert!(idx.refresh());
        assert_eq!(idx.search("记事本", 5, &HashMap::new()).len(), 1);

        // 模拟卸载：重扫后消失
        fs::remove_file(dir.join("豆包.lnk")).unwrap();
        assert!(idx.refresh());
        assert!(idx.search("豆包", 5, &HashMap::new()).is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 条目id跨重扫稳定() {
        let dir = std::env::temp_dir().join("zap_app_index_id_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("豆包.lnk"), "").unwrap();

        let idx = AppIndex::new(vec![dir.clone()], None, false);
        let id1 = idx.search("豆包", 5, &HashMap::new())[0].id.clone();
        fs::write(dir.join("新装.lnk"), "").unwrap();
        assert!(idx.refresh());
        let id2 = idx.search("豆包", 5, &HashMap::new())[0].id.clone();
        assert_eq!(id1, id2, "重扫后同一条目 id 不应漂移");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn 启动次数加权排序() {
        let dir = std::env::temp_dir().join("zap_app_index_frec_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("AA.lnk"), "").unwrap();
        fs::write(dir.join("AB.lnk"), "").unwrap();

        let idx = AppIndex::new(vec![dir.clone()], None, false);
        // 同为前缀命中且等长：无统计时按索引序 AA 在前
        let plain = idx.search("a", 5, &HashMap::new());
        assert_eq!(plain[0].title, "AA");

        // 给 AB 记启动后应排前（加分跨得过同档微差）
        let conn = Connection::open_in_memory().unwrap();
        init_stats(&conn);
        let ab = dir.join("AB.lnk").to_string_lossy().to_string();
        for _ in 0..10 {
            record_launch(&conn, &ab);
        }
        let boosted = idx.search("a", 5, &load_launch_counts(&conn));
        assert_eq!(boosted[0].title, "AB", "常用应用应靠前");

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
