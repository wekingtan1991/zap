use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use rusqlite::{Connection, params};

use super::SearchItem;
use super::icons::{extract_icon_png, extract_uwp_icon, to_data_url};
use super::util::now_ms;
use super::util::wide;

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

#[derive(Clone)]
struct AppEntry {
    name: String,
    path: String,
    /// 应用图标 PNG data URL；未提取/提取失败为 None，前端退回文字角标。
    /// 冷启动首扫只查缓存不现场提取，缺失的由 spawn_icon_backfill 后台补齐
    icon: Option<String>,
    /// 图标缓存键（路径+mtime 哈希）；内存复用与缓存清理共用，省去重算的 stat
    icon_key: u64,
    /// 名称的匹配形态（小写/拼音全拼/首字母），扫描时算一次
    forms: super::MatchForms,
}

impl AppIndex {
    /// 同步全量扫描（诊断/冒烟用；生产路径走 empty+warm 后台化——
    /// 非 test 构建的唯一消费者是 #[ignore] 冒烟测试）
    #[allow(dead_code)]
    pub fn scan(cache_dir: Option<&Path>) -> Self {
        Self::new(
            Self::default_roots(),
            cache_dir.map(Path::to_path_buf),
            true,
        )
    }

    fn default_roots() -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(p) = std::env::var("APPDATA") {
            roots.push(PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"));
        }
        if let Ok(p) = std::env::var("PROGRAMDATA") {
            roots.push(PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs"));
        }
        roots
    }

    /// 空表初始化（评审 P2-02/D2 索引后台化）：entries 空表起步，首扫挪后台
    /// 线程（warm 模式）——setup 不再被目录树+UWP COM+图标缓存读阻塞，
    /// 冷启动热键/托盘就绪优先。last_refresh 记 None：呼出时 refresh_if_stale
    /// 天然触发补扫（后台首扫未完成时 refreshing 门跳过，无并发叠加）
    pub fn empty(cache_dir: Option<&Path>) -> Self {
        Self {
            roots: Self::default_roots(),
            cache: cache_dir.map(Path::to_path_buf),
            uwp: true,
            refreshing: AtomicBool::new(false),
            last_refresh: Mutex::new(None),
            entries: RwLock::new(Arc::new(Vec::new())),
        }
    }

    /// 后台首扫（评审 P2-02/D2）：empty 起步后调用——目录树+UWP COM 在后台
    /// 线程跑完经 refresh 热替换（Arc swap，搜索无锁切换），图标 backfill 挂
    /// 扫描完成回调。耗时落 diag（P2-01 基线纪律：先拿数据再谈优化收益）
    pub fn warm(self: &std::sync::Arc<Self>) {
        let this = std::sync::Arc::clone(self);
        std::thread::spawn(move || {
            let t0 = Instant::now();
            this.refresh();
            super::diag::log(&format!("index\t首扫耗时 {}ms", t0.elapsed().as_millis()));
            this.spawn_icon_backfill();
        });
    }

    #[allow(dead_code)] // 单元测试构造入口（非 test 构建无消费者）
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

    /// 后台补齐缺失图标：快照当前缺图标条目 → 逐个提取（UWP 走
    /// extract_uwp_icon，普通路径走 extract_icon_png）→ 成功的按
    /// 「索引 + 路径比对」回写并热替换条目表，磁盘缓存同步落盘（下次扫描
    /// 即为暖路径）。提取不出（None）的条目跳过不重试，与既有语义一致。
    /// 搜索不受影响：缺图标条目照常参与匹配，前端退回文字角标
    pub fn spawn_icon_backfill(self: &std::sync::Arc<Self>) {
        let this = std::sync::Arc::clone(self);
        std::thread::spawn(move || {
            let pending: Vec<(usize, String, u64)> = {
                let es = this.entries.read().expect("app index poisoned");
                es.iter()
                    .enumerate()
                    .filter(|(_, e)| e.icon.is_none())
                    .map(|(i, e)| (i, e.path.clone(), e.icon_key))
                    .collect()
            };
            if pending.is_empty() {
                return;
            }
            let mut fills: Vec<(usize, String, Option<String>)> = Vec::new();
            for (i, path, key) in pending {
                let png = if path.starts_with("shell:") {
                    extract_uwp_icon(&path)
                } else {
                    extract_icon_png(&path)
                };
                if let Some(png) = png {
                    if let Some(dir) = &this.cache {
                        let _ = fs::write(dir.join(format!("{:016x}.png", key)), &png);
                    }
                    fills.push((i, path, Some(to_data_url(&png))));
                }
            }
            if fills.is_empty() {
                return;
            }
            let mut guard = this.entries.write().expect("app index poisoned");
            // 路径比对防串版本：提取期间 refresh 可能已热替换条目表，
            // 同索引不同路径（应用已卸载）的条目不回写
            let latest = Arc::make_mut(&mut guard);
            for (i, path, icon) in fills {
                if latest.get(i).is_some_and(|e| e.path == path) {
                    latest[i].icon = icon;
                }
            }
        });
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
                    pinned: false,
                    score,
                }
            })
            .collect()
    }

    /// 最常用应用条目（空面板呼出的「呼出 → Enter 即启动」路径）：只收真实
    /// 启动过的（launches>0），按启动次数降序、同数次序稳定（索引序）。
    /// 展示顺序即数组序，score 仅作标记
    pub fn top_launch_items(
        &self,
        launches: &HashMap<String, i64>,
        limit: usize,
    ) -> Vec<SearchItem> {
        let entries = self.entries.read().expect("app index poisoned").clone();
        let mut scored: Vec<(i64, usize)> = entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let n = launches.get(&e.path).copied().unwrap_or(0);
                (n > 0).then_some((n, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.truncate(limit);
        scored
            .into_iter()
            .enumerate()
            .map(|(rank, (_, i))| {
                let e = &entries[i];
                SearchItem {
                    id: format!("app:{:016x}", super::stable_key(&e.path)),
                    title: e.name.clone(),
                    subtitle: e.path.clone(),
                    kind: "app".into(),
                    target: e.path.clone(),
                    icon: e.icon.clone(),
                    pinned: false,
                    score: 55.0 - rank as f32,
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

/// open_item 记一次启动：按 target（lnk 路径 / shell:AppsFolder AUMID）计数
pub fn record_launch(conn: &Connection, target: &str) {
    let _ = conn.execute(
        "INSERT INTO app_stats(target, launches, last_at) VALUES(?1, 1, ?2)
         ON CONFLICT(target) DO UPDATE SET launches=launches+1, last_at=excluded.last_at",
        params![target, now_ms()],
    );
}

/// 全量加载启动计数（表里只有真实启动过的应用，通常几十行）
pub(crate) fn load_launch_counts(conn: &Connection) -> HashMap<String, i64> {
    // prepare_cached（审计 G1）：空面板呼出每次走这里
    let Ok(mut stmt) = conn.prepare_cached("SELECT target, launches FROM app_stats") else {
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
            let icon = icon_cached(cache, icon_key, mem);
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

/// 图标取数（仅缓存）：内存复用 → 磁盘缓存；未命中返回 None 交由
/// spawn_icon_backfill 后台补齐，首扫不再现场提取——首次运行冷提取数百
/// 图标可达秒级，会拖慢 setup 与热键/托盘就绪
fn icon_cached(cache: Option<&Path>, key: u64, mem: &MemIcons) -> Option<String> {
    if let Some(hit) = mem.get(&key) {
        return Some(hit.clone());
    }
    let dir = cache?;
    let file = dir.join(format!("{:016x}.png", key));
    fs::read(&file).ok().map(|bytes| to_data_url(&bytes))
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

fn scan_uwp(cache: Option<&Path>, mem: &MemIcons, out: &mut Vec<AppEntry>) {
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, IBindCtx,
    };
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, IEnumShellItems, IShellItem, SHCreateItemFromParsingName,
        SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
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

        let path = wide("shell:AppsFolder");
        let Some(folder) = SHCreateItemFromParsingName::<_, Option<&IBindCtx>, IShellItem>(
            PCWSTR(path.as_ptr()),
            None,
        )
        .ok() else {
            super::diag::log_warn("shell:AppsFolder 打开失败，UWP 应用未收录");
            return;
        };
        let Some(items) = folder
            .BindToHandler::<Option<&IBindCtx>, IEnumShellItems>(None, &BHID_EnumItems)
            .ok()
        else {
            super::diag::log_warn("shell:AppsFolder 枚举失败");
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
                // 只查缓存不现场提取：冷提取交由 spawn_icon_backfill 后台补齐
                let icon = icon_cached(cache, icon_key, mem);
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

pub struct AppsPlugin {
    pub index: std::sync::Arc<AppIndex>,
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

    /// 本机真实开始菜单的图标提取产出观察（CI/无图标环境不适用，故 ignore）。
    /// 首扫只查缓存不现场提取：冷提取耗时含后台补齐轮询
    #[test]
    #[ignore = "机器相关冒烟：cargo test --lib -- --ignored --nocapture"]
    fn 真实开始菜单图标提取冒烟() {
        let cache = std::env::temp_dir().join("zap_icon_cache_smoke");
        let _ = fs::remove_dir_all(&cache);
        let t0 = std::time::Instant::now();
        let idx = std::sync::Arc::new(AppIndex::scan(Some(&cache)));
        idx.spawn_icon_backfill();
        // 轮询到「全有图标」或「本轮无新进展」（不可提取的条目就到这里）
        let mut last = idx.stats();
        let (total, with_icon) = loop {
            std::thread::sleep(Duration::from_millis(400));
            let s = idx.stats();
            if s.1 == s.0 || s.1 == last.1 {
                break s;
            }
            last = s;
        };
        let cold = t0.elapsed();
        let t1 = std::time::Instant::now();
        let idx2 = AppIndex::scan(Some(&cache));
        let warm = t1.elapsed();
        let cached = fs::read_dir(&cache).map(|rd| rd.count()).unwrap_or(0);
        println!(
            "图标提取 {with_icon}/{total} 条 · 冷提取（含后台补齐）{cold:?} · 缓存重扫 {warm:?} · 缓存文件 {cached} 个"
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
    fn 最常用应用按启动次数排序且只收启动过的() {
        let dir = std::env::temp_dir().join("zap_app_index_top_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("AA.lnk"), "").unwrap();
        fs::write(dir.join("AB.lnk"), "").unwrap();
        fs::write(dir.join("AC.lnk"), "").unwrap();

        let idx = AppIndex::new(vec![dir.clone()], None, false);
        let conn = Connection::open_in_memory().unwrap();
        init_stats(&conn);
        let pa = dir.join("AA.lnk").to_string_lossy().to_string();
        let pb = dir.join("AB.lnk").to_string_lossy().to_string();
        let pc = dir.join("AC.lnk").to_string_lossy().to_string();
        for _ in 0..3 {
            record_launch(&conn, &pb);
        }
        for _ in 0..1 {
            record_launch(&conn, &pa);
        }
        // 按次数降序；从未启动过的 AC 不出现
        let items = idx.top_launch_items(&load_launch_counts(&conn), 4);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, vec!["AB", "AA"]);
        assert!(items.iter().all(|i| i.kind == "app" && !i.pinned));

        // limit 生效：只取最常用的一条
        for _ in 0..1 {
            record_launch(&conn, &pc);
        }
        let items = idx.top_launch_items(&load_launch_counts(&conn), 1);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "AB");

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
