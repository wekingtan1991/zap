# 搜索热路径锁外打分 · 决策记录（P3.4 已关闭）

> 状态：**已关闭 —— 收益天花板不足，不实施**。本文是决策记录与事实核对，
> 保留未采纳方案供重开时参考；第 3 / 4 / 6 节的 Phase A / B 与 S1-S6 均为**存档**，不代表执行计划。
> 日期：2026-11-09（立项 08:44 → spec 11:30 → 闸门评估 11:37 → 收口 11:51）
> 本文核验标记：✅ 实读/实跑 · 🔍 转述 · ❓ 推断
> 过程：08:44 第三轮立项 → 11:06 P3.3 → 11:30 P3.4 spec → 11:37 闸门评估 → 11:51 收口

## 0. 最终结论（先看这个）

**P3.4 关闭：不改 `Plugin` 契约、不加埋点、不设闸门。** 立项时把「needs_conn 插件在锁内全表扫描打分」列为性能短板，实读后发现该定性不成立，收益天花板低到不值得任何接口级改动：

1. ✅ `needs_conn` 覆写只有 **2 处**（`apps.rs:552` / `clipboard.rs:243`），并非「一批插件」；vault 走独立 `vault_search` 命令，不在通用 plugins 列表。
2. ✅ `clip` 锁在 `engine.rs:315-323` 的循环体内**逐个加、逐个放**，不是跨插件长锁。
3. ✅ 打分池 SQL 写死 **`LIMIT 300`**（`clipboard.rs:49`），加图片 6 行 —— 不是「500 条全表扫描」；`app_stats` 只存真实启动过的应用（几十行）。
4. ✅ watcher 用**独立连接**写库（`watcher.rs:62` `open_db_resilient(&worker_path)`），不参与 `clip` 锁竞争；这把锁没有高频竞争者。
5. ✅ 非 DB 插件（含最慢的 `files` HTTP 250ms、窗口枚举）**早已 `scope.spawn` 并行**（`engine.rs:307-314`）。
6. ✅ 项目**已有**端到端搜索时延测量：`pnpm bench:e2e`（`scripts/bench/e2e.mjs:1-12`），CDP 注入、零源码插桩，测「键入→invoke→回执→DOM commit」四段。`benches/layers.rs:3-6` 亦明说全链路不进微基准、端到端走 CDP 层。

**闸门被撤掉的理由**（11:37 用户质疑后的复核）：
- 静态分析已足以判定收益 ≈ 亚毫秒且无高频竞争者，埋点 + 真机采样 + 4 个拍板问题的**成本高于它要保护的决策价值**（一个几乎肯定不做的改动）。
- 上一版 spec 用「L2 性能门被判噪声」推出「必须专门埋点」是逻辑跳跃：criterion 微基准噪声 ≠ 墙钟计时不可行，且项目本就有 CDP 实测层，新埋点属重复造轮子。
- 上一版 spec 自身两处不一致，已在此更正：① 表格已标「watcher 独立连接」，闸门判据却写「观察 watcher 写库被推迟」（前者成立则后者不可能）；② 把 L2 噪声与「无度量手段」混为一谈。

**闸门什么时候才必要**：改动不可逆/高风险（DB 迁移、线协议变更），或出现用户可感知的实锤症状（真机击键卡顿、watcher 掉条）。P3.4 两条都不占，最坏情况是改了 trait 再 revert。

## 1. 现状实证

| 事实 | 位置 | 标记 |
|---|---|---|
| `Plugin::search(&self, query, conn: Option<&Connection>)`，`needs_conn()` 默认 false | `src-tauri/src/providers/plugin.rs:39-47` | ✅ |
| 注释明确「needs_conn 的插件留在当前线程——rusqlite Connection 非 Sync，不能跨线程」 | `engine.rs:297-300` | ✅ |
| 非 DB 插件进 `scope.spawn` 并行，`handles` 按注册序回槽 | `engine.rs:307-314`, `:324-326` | ✅ |
| needs_conn 插件在**当前线程**逐个：`let conn = clip.lock()` → `p.search(q, Some(&conn))` | `engine.rs:315-323` | ✅ |
| 全仓只有 2 处 `fn needs_conn` 覆写 | `apps.rs:552`、`clipboard.rs:243`（另 2 处是 trait 默认与测试探针） | ✅ |
| `AppsPlugin::search` = `load_launch_counts(conn)`（读 `app_stats` 全表）→ `index.search(..., 8, &launches)`（打分） | `apps.rs:555-558` | ✅ |
| `ClipboardPlugin::search` = `search(conn, query, 6)`（历史/图片两表扫描 + `score_lowered` 打分 + 排序 + truncate） | `clipboard.rs:246-250`、`:81-123` | ✅ |
| 空面板路径另有一次 `assemble_opening(&self.plugins, &conn)` + `load_launch_counts`，同在锁内 | `engine.rs:120-130` | ✅ |
| 注册 12 个插件，顺序即展示顺序 | `engine.rs:63-77` | ✅ |
| `panic = "abort"`，join 兜底不是 panic 隔离边界 | `engine.rs:298-300` | ✅ |
| watcher 用**独立连接**写库（不共用 `clip` 锁） | `watcher.rs:62` `open_db_resilient(&worker_path)` | 🔍 |

时序（每次非空 query 的一次 `search`）：

```
probe_all（廉价字符串识别，无锁）
   ↓
run_parallel
 ├─ scope.spawn × N（非 DB 插件：files HTTP / winsys 窗口枚举 / web / settings ...）
 └─ 当前线程串行：
      lock → AppsPlugin::search（SQL + 打分）→ unlock
      lock → ClipboardPlugin::search（SQL + 打分 + 排序）→ unlock
   ↓
join 全部 handles → 按注册序回槽 → 扁平返回
```

锁竞争的**真实影响面**不是「搜索变慢」，而是「这 2 段持锁期间，vault 读写 / 设置保存 / `clip_pin` 等所有走 `conn()` 的命令排队」。这一点才是值得度量的对象。

## 2. 约束与不变量（改动必须守住）

- **C1** `rusqlite::Connection` 非 `Sync`：`&Connection` 不能送进 scoped 线程；但 `Connection` 本身 `Send`，可被单线程独占。
- **C2** 注册顺序 = 结果展示顺序（`plugin.rs:34-35`），`slots[i]` 回槽机制不能变。
- **C3** `panic = "abort"`：线程内 panic = 进程退出，不能靠 `join` 兜底；任何并行化不得引入新的 panic 面。
- **C4** 插件开关是静态 `AtomicBool`，热路径只读原子量、不得查库（`plugin.rs:13-16`）。
- **C5** 剪贴板历史上限 `clip_max` 默认 500；vault 字段形态缓存 `FORMS_CACHE` 与 `PWD_CACHE` 的锁序（clip 连接锁 → 缓存锁，单向）不得反转。
- **C6** 错误口径：命令层 `Result<_, String>` + `diag::log`，不把 OS 原文透出安全边界。
- **C7** 交互铁律：不新增需记忆的组合键；前端搜索防抖与 `cancelled` 守卫不动（🔍 行号来自上一轮体检的子 agent 报告，本轮未复核，故不引用具体行号）。

## 3. Phase A：先度量（建议本轮后续只做这个）

目标：拿到「每次击键 `clip` 锁持有时间」与「其他命令的排队等待」的真实分布，再决定要不要动接口。

**A1 诊断埋点（最低侵入）**：在 `run_parallel` 的 needs_conn 循环内，用 `Instant` 包住 `lock()` 等待与 `p.search(...)` 执行，按插件 id 落 `diag::log`（项目已有 `providers/diag.rs`）。字段建议：
`search\tdb_plugin=<id>\tlock_wait_us=<n>\thold_us=<n>\trows=<n>`。
要求：只在 `debug_assertions` 或显式诊断开关下开启，避免常态日志放大；计时本身不得引入新锁。

**A2 真机采样**：呼出面板连打 20 键、历史接近 500 条、同时触发一次 vault 复制或设置保存，取 `zap.log` 中 `hold_us` 的 P50/P95/P99 与排队命令的等待。

**A3 决策闸门（建议阈值，待用户确认）**：
- `hold_us` P95 **< 2ms** 且未观察到 watcher 写库/设置保存被明显推迟 → **关闭 P3.4**，只在 `plugin.rs` 注释里写明「锁逐个加放、needs_conn 只有 2 个」的纪律，不改契约。
- P95 **≥ 2ms** 或观察到写命令排队 > 10ms → 进入 Phase B，按下表选方案。
- 阈值是 ❓ 工程建议，不是既有共识；用户可改。

> 为何不直接用 criterion：`benches/layers.rs` 是确定性纯层基准，测不到锁竞争与 WAL 写者；L2 +35% 门在本机 A/A 全拦已被判噪声。诊断日志 + 真机采样比微基准更贴近用户实际手感。

## 4. Phase B：候选方案对比（仅在闸门通过后实施）

| 方案 | 做法 | 改动面 | 风险 | 评价 |
|---|---|---|---|---|
| **A 两阶段 fetch / rank（推荐）** | `Plugin` 增 `fn fetch(&self, conn) -> OwnedRows`（默认空）与 `fn rank(&self, q, owned) -> Vec<SearchItem>`；引擎在锁内只调 `fetch` 取 owned 行，出锁后 `rank` 打分/构造 | trait + `apps` / `clipboard` 两实现 + `run_parallel` + 测试，约 150-200 行 | 中：owned 行结构要覆盖两类异构数据，可用 `Box<dyn Any + Send>` 或给 trait 加关联类型（后者波及 12 个实现，不推荐） | 直接命中「锁内打分」根因，且让 DB 插件也能与其他插件并行打分 |
| **B 每插件独立只读连接** | 引擎按 `db_path` 为 2 个 needs_conn 插件各开一个 `Connection`，插件整体进 `scope.spawn` | 连接生命周期、打开失败降级、WAL 读并发、与 watcher 写竞争；`SearchEngine::new` 与字段 | 高：多连接故障面、连接泄漏、读事务一致性 | 并行度最高，但为 2 个轻查询引入连接池语义，性价比低 |
| **C 内存快照** | 后台维护 clipboard/apps 的 `Arc<RwLock<Vec<_>>>`，击键完全不查库 | 快照失效（每次复制/删除/置顶）、内存占用、冷启动一致性 | 高：状态同步复杂度，容易引入陈旧结果 | 收益大但偏离「DB 是唯一事实源」，不建议 |
| **D 仅注释/纪律** | 不改代码，把「逐个加锁、只有 2 个 needs_conn」写进注释 | 0 | 无 | 闸门不通过时的收口 |

**推荐**：闸门通过选 **A**，且**分两步落地**：
1. 先只改 `ClipboardPlugin`（数据量大、打分重），`apps` 保持现状；用 `#[cfg(test)]` 锁住结果顺序与打分不变。
2. 再评估 `apps` 是否值得同样处理。这样每步都有可回滚的独立提交。

## 5. 架构决策（Phase 1 口径）

| 决策项 | 选择 | 理由 |
|---|---|---|
| 项目结构 | 按功能组织，改动收敛在 `providers/plugin.rs` / `engine.rs` / 两个 provider | 与既有按域分模块一致，不引入新层 |
| 接口契约 | 保留 `search` 作为默认路径，新增 `fetch`/`rank` 为**可选覆写**（默认实现回退到 `search`） | 12 个插件零改动，向后兼容；避免关联类型波及全表 |
| 错误处理 | 维持 `Result<_, String>` / `Vec<SearchItem>`，`fetch` 失败按空集降级并落 `diag::log_err` | 与项目「坏库软着陆」口径一致，不把错误抛给前端 |
| 并发 | 不新增线程；打分仍在 `run_parallel` 现有 scope 内完成 | `panic=abort` 下不扩大 panic 面（C3） |
| 实时/认证/API 客户端 | 不适用（本机 Tauri IPC，无网络认证） | — |

## 6. 实施分期（闸门通过后）

- **S1 埋点**（Phase A）：`engine.rs` + `diag`，独立提交 `perf(search): 热路径锁耗时诊断`。
- **S2 采样与决策**：真机采集 P50/P95，由用户拍板是否进入 S3。
- **S3 契约**：`plugin.rs` 加默认实现 `fetch`/`rank`，`engine.rs` 在锁内只 `fetch`、出锁 `rank`；先接 `ClipboardPlugin`。
- **S4 回归锁**：
  - `run_parallel` 输出顺序与串行版逐条一致（已有 `plugin_tests::并行插件执行保持注册顺序且真并行`）。
  - 新增「`fetch` 在锁内、`rank` 在锁外」的静态/行为断言（例如注入一个记录持锁状态的测试插件）。
  - clipboard 打分结果、truncate(6)、固定优先、拼音召回全部沿用既有 `clipboard::tests`。
- **S5 门禁**：fmt / clippy `-D warnings` / `cargo test --lib` / vitest / tsc / check:kb / check:secrets / verify-version / `pnpm smoke`（真机侧）。
- **S6 复测**：按 A2 同样采样，确认 `hold_us` 下降且无功能回归；若收益不足则回滚 S3（保留 S1 埋点）。

## 7. 回滚

- 每个 S 步独立提交，`git revert` 可单独撤销。
- `fetch`/`rank` 的默认实现回退到 `search`，即使中途停摆也不会破坏其余 10 个插件。
- 埋点若被判日志噪声，直接移除 `diag::log` 调用即可，不改变任何行为。

## 8. 关闭决定与重开触发条件

**决定（2026-11-09 11:51，用户「按这个收口」）**：P3.4 关闭。不做锁外打分、不加埋点、不动 `Plugin` 契约；原 4 个待拍板问题一并撤销。产出仅为本文状态更新与两处实读注释（约 10 行），独立一笔提交。

**已落地的收口动作**：
- 本文状态改为「已关闭」，第 3 / 4 / 6 节标注为未采纳方案存档（不删，供重开时参考）。
- `plugin.rs` `needs_conn` 与 `engine.rs` `run_parallel` 注释补 4 条实读结论：仅 2 个 needs_conn、锁逐个加放、watcher 独立连接、打分池 300 行上限。
- 第三轮剩余精力转 **P3.5**（`App.tsx:138-1169` 按域切分）。

**重开触发条件（满足任一才重新评估）**：
1. 真机出现可复现的**击键卡顿**，且 `pnpm bench:e2e` 的 M3 搜索段显著劣化；
2. `needs_conn` 插件增至 **5 个以上**（当前 2 个）；
3. `clipboard::RETENTION_LIMIT` 或打分池上限提高到**数千行**量级；
4. 出现 `clip` 锁的高频竞争者（例如某命令改为每击键走 `conn()`）。

> 重开需按第 8 节触发条件重新评估并经用户拍板，不因本文存档的候选方案自动立项。
