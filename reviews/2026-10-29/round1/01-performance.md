# zap 十角色评审 · 第一轮 · 性能维度（2026-10-29）

评审人：启动与运行时性能工程师。事实源 = 磁盘当前状态（含 lib.rs / clipboard.rs / App.tsx 等在途未提交改动，按简报视为最新事实审）。只读评审，每条发现带 file:line 证据。

---

## 1. 角色结论摘要

1. **线程拓扑与锁纪律整体健康**：搜索/重 IO 全部 `spawn_blocking`（lib.rs:64-74），conn 类插件在 clip 锁内只做纯库查询、HTTP/窗口枚举出锁并行（mod.rs:640-668），stage 的磁盘 IO 全部锁外（stage.rs 模块头 + resolve/finish 拆分），图片入库有界队列+独立 worker（clipboard.rs:240-252）。在途的行号寻址重构已把最大 IPC 项（全文过 IPC）收掉，方向正确。
2. **P1 集中在两处「同类坑未收口」**：① 超级面板恢复剪贴板仍走 `arboard set_image`——本项目在 clipboard.rs:47-53 自己实测过该路径 39MB 图编码 5-6 秒并已为回贴改 DIBV5 直写，超级面板恢复路径漏改；② 剪贴板图片与文本共用 500 条保留上限、单图上限 16MB，磁盘理论可达 GB 级且无 VACUUM。
3. **主线程消息泵仍有两个真实阻塞点**：`open_item` 是同步命令，clip 文本分支的全文本点查与 `record_app_launch`（SQLite 写）都在主线程（lib.rs:1107-1177）——行号协议把 IPC 里的全文省了，读回却压在主线程；图片分支已用「同步探存在 + 后台取数」的正确模式，文本分支未对齐。
4. **启动链路有一个可收的串行段**：`SearchEngine::new` 的 `AppIndex::scan`（目录树 walk + UWP COM 枚举 + 数百图标磁盘缓存读 + base64）同步跑在 setup 里，呼出热键注册排在其后（lib.rs:1785 → 1837）——冷启动时「进程已起但热键不可用」的窗口被拉长。
5. **每击键路径**：`run_parallel` 每插件一个 scoped 线程 + Everything 源 250ms 超时并行 + 应用索引全量打分，属于有意识的成本模型切分，量级可接受；真正的白拿优化在 IPC 重复携带图标 data URL 与 SQLite `synchronous` 缺省。

P0：0 条。P1：3 条。P2：7 条。

---

## 2. 发现清单

### P1-1 超级面板恢复剪贴板走 arboard set_image：大图在剪贴板时，直达热键呼出延迟秒级

**证据**（superpanel.rs:90-111，恢复发生在 `show_panel` 之前）：

```rust
// superpanel.rs:96
} else if let Some(img) = orig_img {
    let _ = super::clipboard::retrying(|| {
        ... cb.set_image(arboard::ImageData { ... })
    });
}
...
// superpanel.rs:111 —— 恢复完成后才展示菜单
show_panel(app, x, y, text.trim().to_string());
```

对照本项目对同一路径的实测结论（clipboard.rs:47-50）：

```rust
/// 剪贴板图片直写 CF_DIBV5，绕开 arboard set_image——它在 DIBV5 之外还用
/// image 库编码一份 PNG 格式，39MB 长截图仅编码就要 5-6 秒（「仅复制没反应」
/// 的实测根因 ...
```

**生产后果**：剪贴板持有截图（`orig_text` 为 None → 先 `get_image` 全量解码 RGBA，superpanel.rs:65-71）且用户对选中文本触发超级面板直达热键：先读全图（4K 截图 RGBA ≈ 130MB 分配+拷贝），捕获成功后再走 set_image 的 PNG 重编码（秒级，量级有本项目自己的实测注释背书）。期间热键看似无响应，且窗口期内用户剪贴板内容是刚注入的选中文本。触发条件常见：截屏后选中一段文字按超级面板。

**建议与权衡**：恢复分支改用已有的 `set_image_dib(w, h, rgba)`（毫秒级，clipboard.rs:54，回贴路径已在用并测试锁定字节布局），约 10 行改动；或者把恢复现场移到 `show_panel` 之后并行。工作量小·风险低（DIBV5 直写已有生产验证）。置信度：**高**（代码路径确定；延迟量级由项目自身实测注释佐证）。

### P1-2 剪贴板图片与文本共用 500 条保留上限、单图 16MB，磁盘可达 GB 级且无 VACUUM

**证据**：

```rust
// clipimage.rs:16 —— 单图上限
pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
// clipimage.rs:140-144 —— 图片清理复用文本的 RETENTION_LIMIT
let _ = conn.execute(
    "DELETE FROM clipboard_images WHERE pinned=0 AND id NOT IN (
         SELECT id FROM clipboard_images ORDER BY created_at DESC LIMIT ?1)",
    params![super::clipboard::RETENTION_LIMIT.load(Ordering::Relaxed)],
);
// appconfig.rs:7 —— 默认 500
pub const DEFAULT_CLIP_MAX: i64 = 500;
```

全仓库无 `VACUUM` / `auto_vacuum` / `synchronous`（findstr 全量核对，clipboard.rs:202-218 的 open_db 仅设 WAL）。图片行删除后 BLOB 占用的页留在空闲链表，DB 文件只增不缩。

**生产后果**：重度截图用户（日常截图工作流）稳态磁盘占用 = 保留图片数 × 单图 PNG 体积，500 × 数 MB = 数 GB；理论上限 500 × 16MB = 8GB。行数有界（≤500+500）所以查询不慢，代价纯粹是磁盘与用户信任（「一个启动器为什么占了 5GB」），以及 WAL checkpoint 与备份（config 导出不含库，但用户手动拷 AppData 目录）时的长尾。

**建议与权衡**：① 图片表单独设保留上限（如 50 条）或按总字节预算（如 256MB）淘汰——一行常量+一处 SQL，工作量小；② `open_db` 迁移到 `PRAGMA auto_vacuum=INCREMENTAL`（需一次性 VACUUM 重建库，注意旧库迁移顺序）或定期（如启动时低频）`VACUUM`。工作量小-中·风险低（迁移路径要测旧库升级）。置信度：**高**（数学确定；实际触发分布中等，取决于用户截图频率）。

### P1-3 open_item 同步命令在主线程做全文本点查与 SQLite 写

**证据**（lib.rs:1107 同步命令 = Tauri 主线程执行，项目自己的军规注释 lib.rs:114-116 明确这一前提）：

```rust
// lib.rs:1107 —— 同步命令，主线程
fn open_item(app: AppHandle, item: SearchItem) -> Result<(), String> {
    ...
    if item.kind == "app" {
        app.state::<AppState>().engine.record_app_launch(&item.target);   // SQLite 写（WAL commit fsync）
    }
    ...
    let clip_text = if item.kind == "clip" {
        Some(engine.clip_row(rowid).ok_or("该条剪贴板已被清理".to_string())?) // 全文点查，主线程
```

图片分支已经是正确模式（lib.rs:1177 `clip_image_exists` 同步探存在 + lib.rs:1194 `std::thread::spawn` 里解码），文本分支没有对齐。

**生产后果**：行号协议重构（ad99093）的动机场景「几 MB 的长文本」下，选中即粘贴时主线程要完成 SQLite 全文读 + String 拷贝（数 MB ≈ 数-数十 ms）+ 一次 WAL fsync，期间消息泵停摆，`win.hide()` 的收面板动画与失焦事件被推迟。常规 KB 级条目无感——这是一个「极端输入下长尾」类问题，而非平均延迟问题。

**建议与权衡**：文本分支复用图片分支的两段式——同步只做一行 `clip_row_exists`（PK 探存在，保「已被清理」的同步报错语义），全文读回挪进 `paste_text_async` 已有的后台线程（该线程本来就吃 `text` 参数，只需把 rowid 传进去在锁外点查）。工作量小·风险低。置信度：**高**（路径确定；量级依赖用户剪贴板内容分布）。

### P2-4 启动 setup 同步扫描应用索引，呼出热键注册排在后面

**证据**（lib.rs:1785 → mod.rs:188 → apps.rs:47-87）：

```rust
// mod.rs:188 —— setup 内同步执行
let index = std::sync::Arc::new(apps::AppIndex::scan(icon_cache.as_deref()));
// apps.rs:81-85 —— walk 开始菜单目录树 + UWP COM 枚举 + 每条 MatchForms + 图标磁盘缓存读
for root in roots {
    walk(root, 0, cache, mem, &mut entries);
}
if uwp {
    scan_uwp(cache, mem, &mut entries);
}
```

随后才是热键注册（lib.rs:1837-1849）与托盘。图标**提取**已移后台（`spawn_icon_backfill`，mod.rs:189），但扫描本体（数百个小 PNG 读盘+base64、`shell:AppsFolder` COM 枚举）仍在主线程 setup。

**生产后果**：登录自启场景下，从进程起来到热键可呼出之间多出几十至数百 ms（机器/应用数相关，未实测）。用户感知 =「开机后马上按热键没反应」。UWP 枚举在最坏情况（系统刚登录、shell 服务忙）可能更久。

**建议与权衡**：`AppIndex::scan` 移入后台线程——`entries` 以空 `Arc<Vec>` 初始化，扫完走 refresh 已有的热替换（apps.rs:105）写入；`spawn_icon_backfill` 挂在扫描完成回调上。权衡：极早期呼出时应用列表为空（1-2s 内自愈，其余源不受影响）。工作量小·风险中低（时序：须保证 backfill 在首扫完成后跑）。置信度：**中**（代码路径确定，量级未实测）。

### P2-5 每击键搜索响应重复携带图标/缩略图 data URL 过 IPC，前端 key 含 index 破坏复用

**证据**：

```rust
// apps.rs:227 —— 每次搜索为每条 app 结果克隆完整 base64 data URL
icon: e.icon.clone(),
// clipimage.rs:173-176 —— 每次搜索为图片条目重编码缩略图 base64
icon: Some(format!("data:image/png;base64,{}",
    base64::engine::general_purpose::STANDARD.encode(thumb))),
```

```tsx
// App.tsx:1286 —— key 混入下标，列表重排即整行重建
key={it.id + String(i)}
```

**生产后果**：连续输入时每击键 ≈ 8 app 图标 + 若干缩略图的 JSON 序列化/反序列化（单个 64px PNG base64 约 3-8KB，每击键约数十 KB）；数据 URL 字符串每次都是新分配，`<img src>` 无法命中 WebView 解码缓存。绝对量无感，但属高频路径上的纯重复功。

**建议与权衡**：轻量版——前端按 `it.id` 建 `Map<string, string>` 图标缓存（或后端图标引用化：`icon` 只在 id 首见时携带）。再把 key 去掉 index（id 冲突时才追加序号，如 `clip:*` 与 `clipimg:*` 撞号的已知场景改用 `kind:id`）。工作量小·风险低。置信度：**中**（收益为估算）。

### P2-6 每击键 run_parallel 为每个插件 spawn 一个 scoped 线程

**证据**（mod.rs:648-652）：

```rust
for (i, p) in plugins.iter().enumerate() {
    if plugin_off(p.as_ref()) || p.needs_conn() {
        continue;
    }
    handles.push((i, scope.spawn(move || p.search(q, None))));
}
```

**生产后果**：11 个插件（排除停用与 needs_conn 后约 7-9 个）× 每击键的线程创建/销毁（各 ~10-50µs）+ Tokio blocking 池瞬时占用；快速连打时在途 3-5 个搜索 × 10 线程。是刻意设计（注释：单插件 panic 只损失该源、保注册序），当前量级可接受，不改不算债。

**建议与权衡**：维持现状。若未来插件数增长或要压低端机，可换固定 worker 池 + 每击键分发，但那是复杂度换微秒级收益——现在不建议做。置信度：**高**（事实）/ 低（是否值得改）。

### P2-7 winsys 窗口快照枚举持全局 Mutex，快照过期后的首个击键串行等待

**证据**（winsys.rs:23-33）：

```rust
fn snapshot() -> Arc<Vec<WinEntry>> {
    let mut guard = SNAPSHOT.lock().expect("win snapshot poisoned");
    if let Some((at, list)) = guard.as_ref() && at.elapsed() < SNAPSHOT_TTL {
        return list.clone();
    }
    let list = Arc::new(enumerate());   // 持锁期间枚举全部可见窗口 + 逐窗口取进程名
    *guard = Some((Instant::now(), list.clone()));
    list
}
```

**生产后果**：TTL 1.5s 过期后的那次击键，所有并发搜索线程的 winsys 源在 Mutex 上排队等 `enumerate()`（EnumWindows + 每窗口 `process_stem_of_pid`，典型 1-10ms；`GetWindowTextW` 对跨进程窗口读缓存不挂起，无病态挂死通路）。后续 1.5s 内击键免费。

**建议与权衡**：可接受，不建议改。若要收：`try_lock` + 未过期旧快照兜底（宁旧不等）。工作量小·收益小。置信度：**高**（代码事实）。

### P2-8 SQLite 缺 synchronous=NORMAL：剪贴板每次变更 2 次写 = 2 次 WAL fsync（FULL 缺省）

**证据**（clipboard.rs:202-211 只设 `journal_mode=WAL`，无 synchronous 调整；watcher 每次剪贴板变更执行 INSERT + 保留 DELETE 两笔写，clipboard.rs:280-291；图片入库同样两笔，clipimage.rs:128-144）。

**生产后果**：WAL 模式缺省 `synchronous=FULL`，每次 commit fsync WAL。HDD 机器上每次复制多 ~10-20ms 后台延迟（有界队列容量 4 的时序余量被吃掉，截图风暴下更容易满队丢图）；SSD 无感。写在 watcher/worker 线程，不卡 UI。

**建议与权衡**：`open_db` 加一行 `PRAGMA synchronous=NORMAL`（SQLite 官方对 WAL 的推荐配对，掉电只丢最后一个事务不损坏库——对剪贴板历史可接受，vault/DPAPI 路径加密不依赖 fsync 语义）。一行改动·风险极低。置信度：**高**。

### P2-9 空查询路径每次呼出读最新剪贴板全文，识别却只用前 256KB

**证据**：

```rust
// clipboard.rs:540-543 —— 全文出库
"SELECT id, content FROM clipboard_history ORDER BY created_at DESC, id DESC LIMIT 1"
// mod.rs:693-699 —— 识别只拿它跑 detect
if let Some((rowid, text)) = clipboard::latest(conn) {
    ... items.extend(p.detect(rowid, &text));
```

识别上限：jsontools.rs:110 `t.len() > MAX_ANALYZE_BYTES` 即拒绝、texttools.rs:138 同款——超过 256KB 的部分对识别是纯浪费。

**生产后果**：每次面板呼出（spawn_blocking 线程）多读多拷贝 MB 级字符串；不卡 UI，纯内存带宽与峰值内存浪费（呼出瞬间数 MB 文本常驻）。

**建议与权衡**：`latest` 拆两版——识别用 `substr(content, 1, 262145)`（探超长用长度差判断），快速粘贴/回贴路径保留全文版。工作量小·风险低。置信度：**高**。

### P2-10 三窗口 ×3 份 WebView2 + React bundle 常驻内存（隐藏不销毁）

**证据**：tauri.conf.json:13-56 三个窗口（main/superpanel/stagedrop）全部随应用启动创建（`visible:false` 只是隐藏，webview 与渲染进程照常起）；src/main.tsx 确认三窗口共用同一 dist 入口，各挂自己的根组件。

**生产后果**：常驻启动器的内存基线 ≈ 3 个 WebView2 渲染管线（每个 bundle 解析 + React 树，典型合计 100-200MB RSS 量级）。「无存在感」的代价是常驻内存三倍于单窗口方案。

**建议与权衡**：权衡清楚再动——按需创建辅助窗口会在热键/拖拽首用路径上引入 100-300ms 的 WebView2 冷启动，正是本项目最敏感的呼出延迟。可考虑的中间态：stagedrop/superpanel 保持预建，但换 `url` 指向极薄页面（避免整份 React bundle ×2 的解析）；或接受现状并把它记为已知的内存特性。工作量中·风险中。置信度：**中**（方向确定，具体量级未实测）。

---

## 3. 性能类功能或流程更改建议

1. **超级面板恢复改 DIBV5 直写**（P1-1 的修法落地）：restore 分支换 `set_image_dib`，并把「恢复现场」挪到 `show_panel` 之后并行——菜单首帧不再等恢复完成。顺带给 `trigger_from_hotkey` 加一段耗时 diag 日志（`get_image`→`show_panel` 的总时长），让「热键慢」今后可远程诊断。
2. **open_db 设置档一次到位**：`PRAGMA synchronous=NORMAL` + 评估 `auto_vacuum=INCREMENTAL`，与 P1-2 的图片保留预算一起做（同一个迁移点，一次 VACUUM 重建）。这是全项目性价比最高的一处改动。
3. **clip 存储预算（功能级建议）**：图片表改按「总字节预算 + 条数上限」双阈值淘汰（如 256MB / 50 条，设置页可调），文本表维持 500 条；两个上限分开暴露给用户，文案说清「截图占空间的主要来源」。
4. **呼出链路的诊断埋点**：在 `toggle_panel`（show→emit）与前端 `zap://shown` 处理器（收到→列表就绪）两端打 diag 时间戳，建立「呼出到可交互」的基线数据；本轮 P2-4/P2-5 的收益判断从此有据可依，也是给 1000 陌生用户出事时预留的定位通道。
5. **索引扫描后台化 + 首扫完成事件**（P2-4 落地）：scan 挪后台，完成后 `emit zap://index-ready`；前端无需改动（首帧列表本来就是空查询结果，应用源就绪后自然出现）。若接受「极早期呼出无应用条目」，这是纯收益。

---

## 4. 诚实清单

- **所有时延量级（数百 ms/秒级/10-20ms）均为代码路径 + 本项目既有实测注释的推断，本轮未做真机测量**。P1-1 的 5-6 秒数字直接引自 clipboard.rs 注释（39MB 场景）；4K 常见场景按比例推算 1-2 秒，未实测。
- P2-4 的启动扫描耗时未测：warm 缓存下可能只有几十 ms，冷启动 + 大量 UWP 应用时更久；是否值得后台化建议先按建议 4 加埋点拿基线。
- `run_parallel` conn 类插件在 clip 锁内的实际持锁时长未测；按 SQL 量级推断为毫秒以下，主线程 `record_app_launch` 被它拖住的场景理论存在但未观察到证据。
- 一切 `app.opener()` 调用（ShellExecute）都在同步 `open_item` 主线程上；ShellExecute 对正常路径亚毫秒，但对指向断连网络驱器/慢处理器的 target 可能阻塞主线程——这是 Windows 语义级风险，未构造复现，列入观察。
- 前端 `zap://shown` 后 items 未清空，下一轮呼出的首帧会短暂显示上一轮列表（ms 级，被呼出动画掩盖）；未确认用户可见，未列为发现。
- `stagedrop` 80ms 常驻轮询（stagedrop.rs:125,358-360）的 CPU 占比按 FindWindowW 微秒级推断 <0.1%，未实测；仅续航敏感场景可能可感知。
- P2-10 的三窗口内存量级（100-200MB RSS）是 WebView2 的经验数字，未对 zap 实测 RSS。
