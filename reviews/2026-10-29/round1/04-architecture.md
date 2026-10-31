# 04 · 软件架构评审（第一轮 · 2026-10-29）

评审人角色：软件架构师（十角色交叉评审第一轮，仅架构维度）。
事实源：磁盘当前状态（lib.rs / clipboard.rs / App.tsx 等含在途行号寻址改动，按最新事实审）。
本文遵守共享简报既有纪律：决策提纯（b523d22）、并行边界按成本模型切分、非确定失败不缓存；所有建议均为继承与延伸，无推翻拍板项。

---

## 1. 角色结论摘要

1. **整体骨架是健康的，病灶集中在 lib.rs 一层。** providers/ 24 个模块依赖方向基本单向、插件协议（plugin.rs）边界清晰、决策提纯纪律让 120 个 Rust 测试完全脱离 Tauri 运行；但 crate 根 lib.rs（2024 行，55 个命令 + 窗口生命周期 + 设置编排 + 托盘 + 热键分发混居）是唯一的巨石，且它恰好承载了**历史上三次事故的领域（失焦即隐/Esc）却没有任何测试**。
2. **设置链路是当前最大的结构性负债**：新增一个设置要在 4 个文件改约 10 处（后端 8 处镜像 + 前端 2 处），且「SQLite 存档 → 启动装载 → 运行时原子量」影子缓存有 setup/save 两个手工同步点——漏一处即静默失效，生产标准下属高概率事故源。
3. **并发拓扑没有死锁，但有两处结构性脆弱**：全局 17 个 static 原子量 + 8 个 static Mutex + AppState 6 把锁，其中 `direct_hotkeys` 锁嵌套持有 DB 锁（单向、今日安全、无守卫注释）；Mutex poisoning 走 `expect()` 主线程 panic 路径（15 处），与 stage.rs 已有的 poison-tolerant 写法不一致。
4. **providers 存在两条向上的反向依赖**（stagedrop.rs:889 → `crate::handle_stage_drop`；superpanel.rs:279 → `crate::toggle_panel`），crate 根在当共享胶水——这是拆 lib.rs 前必须先解的边。
5. **前端 App.tsx（1450 行）与 lib.rs 同病但病轻**：View 状态机（App.tsx:60-70）、routeItem 纯函数、registry.tsx 已把方向走对；残余病灶是约 300 行 vault 域动作逻辑栖身 App 根而非 vault 插件模块，以及所有域共享一个键盘处理栈。

---

## 2. 发现清单

### P0-1 lib.rs 巨石：面板生命周期策略（事故家族防线）与 glue 混居在唯一无测试层

**证据**
- lib.rs 2024 行，`invoke_handler` 注册 55 个命令（lib.rs:1722-1778）；`save_settings` 单函数 235 行（lib.rs:347-581）；`setup` 闭包 240 行（lib.rs:1779-2019）。
- 职责混居盘点：命令适配层（约 60% 篇幅）、窗口管理（hide_panel_window lib.rs:79-92 / toggle_panel lib.rs:1460-1505 / open_stage_panel lib.rs:1514-1556）、失焦即隐策略内联在 `on_window_event`（lib.rs:1699-1719）、stagedrop 接线（handle_stage_drop lib.rs:1343-1382）、剪贴板注入编排（paste_text_async lib.rs:1094-1105 / quick_paste_latest lib.rs:1562-1595）、设置编排（lib.rs:225-664）、托盘（lib.rs:1927-2018）。
- 测试分布：16 个文件的 120 个 `#[test]` 全部位于 providers/；lib.rs 零测试。`on_window_event` 里 hold/grace/前台归属三重判定（lib.rs:1706-1718）——简报记录的三次 Esc 事故的同族逻辑——是全仓库最需要回归锁、却唯一没有锁的核心策略。

**架构后果**
拆错顺序的典型伤害：先按行数机械切文件（命令搬运）会把策略层切得更散；而不拆直接加功能（第 5 个直达热键、第 9 个设置）会继续把这 2000 行推向 3000 行，事故面扩大且每次都靠人肉回归。

**建议**
按 §3.1 的五步路线图拆分（每步独立提交、行为不变）。核心动作不是「分文件」而是**沿既有决策提纯纪律把策略层抽成可测纯函数**：失焦即隐判定提为 `should_hide_on_blur(hold, shown_at, fg_in_panel, now) -> bool`（落点 panel.rs，现状语义 = lib.rs:1707-1717 逐分支等价），随后补齐三事故场景的单元测试。这与 stagedrop.rs:291 `decide` 同一手法，是纪律的自然延伸而非新发明。

**权衡**：不拆的风险（持续膨胀 + 策略无回归防线）显著大于拆的成本；五步路线每步可独立回滚。
**置信度：高**（证据全部来自逐行阅读）。

---

### P0-2 设置链路「九处镜像」：新增一个设置 = 4 文件约 10 处改动 + 2 个手工同步点

**证据**（以 `stagedrop_open_panel` 为样本逐处核实）
1. `src-tauri/src/providers/appconfig.rs:40-41` — `KEY_STAGEDROP_OPEN_PANEL` + `DEFAULT_STAGEDROP_OPEN_PANEL`
2. `appconfig.rs:100` — `ConfigSettings.stagedrop_open_panel`（配置导出/导入契约）
3. `src-tauri/src/providers/stagedrop.rs:45` — 运行时 `static AtomicBool`
4. `lib.rs:1820-1824` — setup 启动装载：读 SQLite → store 原子量（同步点一）
5. `lib.rs:247` — `AppSettingsDto` 字段
6. `lib.rs:288` — `build_app_settings` 读原子量
7. `lib.rs:364, 530-536` — `save_settings` 入参 + Option 容错 + store 原子量 + `set_setting` 落库（同步点二）
8. `lib.rs:608` — `config_settings_from` DTO→ConfigSettings 映射
9. `src/views/SettingsView.tsx:88` — `AppSettings` TS 接口字段（另加表单控件与保存载荷两处）

即：DB 存档（真源）、运行时原子量（影子缓存）、DTO（下发）、ConfigSettings（配置文件）四套表示，靠 setup 与 save_settings 两处手工代码保持一致。`appearance`/`clip_autopaste` 等同构可验证。

**架构后果**
漏第 4 处（setup 装载）= 改了设置重启后失效，用户视角「设置保存了但不生效」；漏第 8 处 = 配置导出缺键，导入静默丢设置——两者都是**无报错的静默缺陷**，远程诊断只能靠逐版 diff。每个新设置还要重复一次 Option 容错语义（lib.rs:352,359,362 注释三连），认知负担线性增长。

**建议**（两阶段收口，见 §3.2）
- 阶段一（纯 Rust 内聚，线协议不变）：每个设置的「装载 + 落库 + 影子同步」收进 owner 模块（如 `stagedrop::load_settings(conn)` / `stagedrop::persist(conn, v)`），lib.rs 的 setup 与 save_settings 从「内联镜像逻辑」变为「逐设置调一个函数」；镜像处数从 10 降到约 5，且漏装漏存变成编译期可见（调用清单里少一行）。
- 阶段二（类型契约）：DTO → TS 用 specta/ts-rs 生成，消灭 SettingsView.tsx:69-100 手写镜像；若不想引依赖，最低限度加一条镜像锁定测试（Rust DTO 序列化样本 ↔ 前端 AppSettings 键集合比对，挂进现有 32 个 vitest）。

**权衡**：阶段一改的是组织方式不动行为；阶段二引依赖与单人维护的简洁原则有张力，可只做「锁定测试」低成本版。
**置信度：高**。

---

### P1-3 Mutex poisoning 走主线程 panic：与仓库已有的 poison-tolerant 写法不一致

**证据**
- lib.rs 15 处 `.expect("xxx poisoned")`，其中 4 处在**主线程路径**：`toggle_panel` 的 `prev_foreground`（lib.rs:1473-1476）与 `shown_at`（lib.rs:1493）、`on_window_event` 失焦分支的 `shown_at`（lib.rs:1710-1714）、热键回调的 `hotkey`/`direct_hotkeys`（lib.rs:1647,1658）。主线程 panic = 整个应用退出；此后每次呼出/失焦都重复 panic。
- 对照：stage.rs:278 已采用 `unwrap_or_else(|e| e.into_inner())`（poison 后取内层数据继续），mod.rs:935 测试锁也用同款。同一仓库两种纪律并存。
- diag.rs 无 panic 钩子：主线程 panic 直接死进程，zap.log 里不会有任何痕迹（「远程诊断」场景下的盲区）。

**架构后果**
poisoning 前提是持锁期间 panic——当前临界区小且无 panic 点，概率低；但一旦发生（未来任何人在临界区里加了可能 panic 的代码），后果是**启动器整体不可用且日志无痕**，比单命令失败严重一个量级。

**建议**
1. 统一为 poison-tolerant：主线程路径的 4 处 `expect` 换 `unwrap_or_else(|e| e.into_inner())`（这些锁保护的都是可重置瞬态——shown_at/prev_foreground 取默认值语义完全安全；hotkey 取毒数据等价取旧值）。工作量约 10 行。
2. `run()` 里挂 `std::panic::set_hook` → `providers::diag::log`，补上 panic 痕迹（diag.rs 的既有「诊断落盘」职责延伸）。

**权衡**：poison-tolerant 会掩盖逻辑 bug（毒数据继续用）——但这些锁全是单写者短临界区，风险可接受；日志钩子零成本。
**置信度：高**。

---

### P1-4 并发拓扑盘点与两处脆弱点（嵌套锁无守卫、影子原子量双写者）

**锁清单**（静态盘点，全局量 25 项）

| 载体 | 锁/原子量 | 写者 | 读者 |
|---|---|---|---|
| lib.rs `AppState` | `hotkey`/`direct_hotkeys`/`clip_item`/`shown_at`/`prev_foreground` Mutex + `hold_panel` AtomicBool（lib.rs:40-50） | save_settings/toggle_panel/setup/热键回调 | 同左 |
| mod.rs `SearchEngine` | `clip: Mutex<Connection>`（mod.rs:165，**全 DB 咽喉**） | 所有域命令 | 同左 + needs_conn 插件（mod.rs:654-662 锁内串行） |
| clipboard.rs | `SUPPRESS_CLIP_UNTIL`/`CLIP_ENABLED`/`PASTE_TO_PREV`/`RETENTION_LIMIT` 四原子量（:14,181,185,188） | save_settings/setup/托盘/superpanel/vault | watcher 线程（:269-291） |
| stage.rs | `CAP`/`STAGE_ON` 原子量 + `ICON_CACHE` Mutex + `ICON_EXTRACT_LOCK` Mutex（:20,23,237,266） | save_settings/setup | finish/搜索 |
| stagedrop.rs | `STAGEDROP_ENABLED`/`STAGEDROP_OPEN_PANEL`/`POSITION`/`BAR_SHOWN`/`HOVERING`/`STAGE_VIEW_OPEN` 六原子量 + `STATE: Mutex<DropState>`（:40-190） | 设置路径/setup/**lib.rs:90 越权直写** | 轮询线程（:357-440） |
| winsys.rs | `SNAPSHOT` Mutex（:21，1.5s TTL） | 搜索线程 | 搜索线程 |
| superpanel.rs | `LAST_PAYLOAD` Mutex（:33） | trigger_from_hotkey | superpanel_payload 命令 |
| hotkey_capture.rs | `CAPTURING` + `PENDING`/`STARTED` Mutex（:13-16） | start/stop | 轮询线程 |
| everything.rs / files.rs | `LAST_ATTEMPT`/`PORT`/`LAST_FAIL` Mutex（everything.rs:308; files.rs:13-14） | ensure/probe | 同左 |
| shortcuts.rs | 实例级 `RwLock<Arc<Vec>>` 缓存（:59） | save | 搜索热路径 |

脆弱点一：**嵌套锁**——save_settings 在持有 `direct_hotkeys` 锁期间调用 `engine.get_setting/set_setting`（lib.rs:444 → :448-456,473），即 `direct_hotkeys → clip` 嵌套。当前全仓库无反向边（引擎不知晓 AppState），无死锁；但这条顺序约束只存在于代码里，没有注释或类型表达，未来谁在引擎方法里回调 AppState 就造出反向边。

脆弱点二：**设置类原子量是「影子缓存」但写者分散**——`CLIP_ENABLED` 有三个写者（setup lib.rs:1793 / save_settings lib.rs:497 / 托盘 lib.rs:1984），`STAGEDROP_ENABLED` 两个（setup lib.rs:1819 / save_settings lib.rs:524）。托盘路径（lib.rs:1982-1997）绕过 save_settings 自己落库+翻原子量，是第三套同步逻辑。

**架构后果**
现状可运行（写者均单线程化或低频），真正的风险是**演进风险**：新增设置照抄现有模式会把「三写者」模式复制下去；嵌套锁顺序一旦被破坏，症状是偶发挂死，远程几乎不可诊断。

**建议**
1. lib.rs:444 的 `direct_hotkeys` 锁作用域收紧：锁只护 map 读写，`gs.register/unregister` 与 `engine.set_setting` 移出临界区（每次循环先算好、锁内只 insert/remove）。顺带在锁处加一行「锁序：direct_hotkeys → clip，勿反向」守卫注释。
2. 设置类原子量的写者收敛到 owner 模块（与 P0-2 阶段一同一动作：`clipboard::apply_clip_settings(conn, ...)` 之类），setup/save/托盘三处调同一函数。
3. 其余锁（SNAPSHOT/ICON_CACHE/LAST_PAYLOAD 等缓存型）维持现状——单写者或 TTL 语义下 Mutex 足够，换 RwLock/parking_lot 属过度设计。

**置信度：高**（清单经逐文件核对；「无反向锁序边」的判断见诚实清单）。

---

### P1-5 providers 的两条反向依赖：crate 根在当共享胶水

**证据**
- `stagedrop.rs:889`：OLE Drop 回调里 `crate::handle_stage_drop(&app, paths)`——providers 内部模块向上调用 crate 根函数。
- `superpanel.rs:279`：`crate::toggle_panel(app, Some(text))`——同型反向边。
- 反向的镜像：`lib.rs:90` 在 `hide_panel_window` 里直写 `providers::stagedrop::STAGE_VIEW_OPEN.store(false, ...)`——crate 根越权改 providers 内部状态机标志（stagedrop.rs:190 注释表明该量语义属 stagedrop 状态机）。
- 其余 22 个 providers 模块零 tauri 依赖（grep 证实仅 stagedrop/superpanel 引 `tauri::`），边界本来很干净。

**架构后果**
这三条边使 lib.rs 无法被拆分：动 handle_stage_drop/toggle_panel 的位置必断 providers 编译；反之改 stagedrop 状态机（如收敛为单枚举）也必须同时改 crate 根。它们是目前「providers 无关 lib.rs」这一好性质的仅有的三个破口。

**建议**
拆分路线图的第 0 步（先于任何文件移动）：
1. `handle_stage_drop` 下沉为 `providers::stagedrop::handle_drop`（或独立 `stage_flow` 模块），lib.rs 命令层与 wry 兜底路径都调它——stagedrop.rs:889 的 `crate::` 引用自然消失；
2. superpanel 的 `open_panel` 动作改为注入回调或调用新的 `panel::toggle_panel`（第 3 步产物）；
3. lib.rs:90 直写改为调用 stagedrop 已有的 `set_stage_view(app, false)` 之外的专用入口 `notify_panel_hidden(app)`（语义 = 面板已隐，状态机自行清 STAGE_VIEW_OPEN，与 lib.rs:88-90 注释意图一致）。
三处均为等价改写，行为不变。

**置信度：高**。

---

### P1-6 App.tsx（1450 行）与 lib.rs 同病：vault 域逻辑栖身 App 根

**证据**
- 单组件 `AppView` 覆盖 App.tsx:149-1516；已做对的：`View` 联合状态机（:60-70，注释明言「取代原先 8 个并列布尔」）、`routeItem` 纯分发（plugins/dispatch.ts）、`kindUi` 注册表（plugins/registry.tsx）、ErrorBoundary（:108-147）、窗口捕获键盘纪律（:812-850）。
- 残余病灶：vault 域动作全在 App 根——`vaultAction`/`copyField`/`startEdit`/`openDetail`/`openDeleteConfirm`/`pinCurrent`/`deleteCurrent`/`editCurrent`/`refreshVault`/`formSaved`/`cancelForm` 约 300 行（App.tsx:492-1020），而 vault 的视图组件已在 plugins/vault/ 独立。域逻辑与域视图分离在两个层级。
- 键盘栈集中：主面板 onKeyDown（:1033-1107）内嵌保护域 Alt+1..6 分支、次级操作分支、导航分支——新域接入继续往这一个函数里加分支。

**架构后果**
前端不加新域则此病不恶化；一旦加域（或 vault 加动作），App 根继续膨胀且 actions 装配（:912-927）的依赖数组（:970 有 eslint-disable）成为 stale-closure 温床。与后端 lib.rs 的差异：前端已有正确的「抽离方向与契约层」，缺的只是下一刀。

**建议**
单步可提交：把 vault 域动作提为 `plugins/vault/useVaultActions.ts`（入参 `invoke` 包装 + `refreshVault` 回调，返回与现 `actions.vaultPin` 等同的接口），App.tsx 降至装配层。行为不变、无 UI 改动；vitest 现有 dispatch 测试不动。键盘栈暂不动（保护域组合键属交互拍板范畴，等第二域出现再抽象，避免过早泛化）。

**置信度：中高**（未逐行核对 vault 视图组件的 props 反向依赖，见诚实清单）。

---

### P2-7 stagedrop：不建议现在收敛为单枚举；建议「外部写者收口 + BAR_SHOWN 并入 STATE」的小步演进

**证据**
- 状态分散：`STATE: Mutex<DropState>`（stagedrop.rs:172，session_active/last_seen/gone_since/feedback/generation 五字段）+ 4 个机器态原子量（`BAR_SHOWN`:121 / `HOVERING`:185 / `STAGE_VIEW_OPEN`:190 / `POSITION`:87）。
- `decide` 纯函数（:291-322）+ 10 情形测试（:977-1050）已把决策核心锁死；`hide_for_handoff`（:226-235）刻意「藏窗口不清会话」的时序语义有真机实测背书（:219-225 注释）。
- 外部写者三处：lib.rs:90 直写 STAGE_VIEW_OPEN；lib.rs:1355/1369 调 begin/finish_processing；lib.rs:1533 调 hide_for_handoff。

**架构后果**
大爆炸式单枚举会重写 decide 与全部写者，正是三次真机调优（抖动去抖/悬停交织/移交时序）最容易回退的地方；收益（类型收窄）低于回归风险。真正的脏点是外部写者绕过状态机边界。

**建议**
1. 先做 P1-5 的外部写者收口（所有跨模块写经 stagedrop 公开 API）；
2. `BAR_SHOWN` 并入 `STATE`（该量的读侧轮询线程与写侧 set_stage_view/hide_for_handoff/ensure_visible 全都已锁 STATE 或可廉价合并；`relayout_if_shown`:112-116 的无锁读改为短暂锁 STATE，80ms 节奏下无竞争顾虑）；
3. `POSITION` 与两个 ENABLE 开关保留原子量——它们是配置不是机器态，设置路径低频写。
完成 1+2 后，机器态只剩 STATE 一处，「未来若要单枚举」变成纯内部重构。

**置信度：中高**（BAR_SHOWN 并入的竞态分析基于静态阅读，未动态验证）。

---

### P2-8 SearchEngine 是 40+ 方法的门面 + `clip: Mutex<Connection>` 单咽喉——可接受，但要有「膨胀止损线」

**证据**
- `SearchEngine`（mod.rs:163-572）聚合 search/vault×6/stage×8/clip×7/settings/shortcuts/plugin 管理约 40 个 pub 方法；构造函数装配 11 个插件（mod.rs:197-209）。
- 全部库操作共享一把 `clip: Mutex<Connection>`（mod.rs:165）；注释（mod.rs:656-660）明示 needs_conn 插件锁内串行、其余并行——与「并行边界按成本模型切分」纪律一致，且有并行保序测试（mod.rs:991-1018）。
- watcher 与图片 worker 各持独立连接（clipboard.rs:240-260），WAL 下并存——设计正确。

**架构后果**
门面本身无害（Tauri 命令层需要单一入口）；风险在两处：① 每个新域继续往 SearchEngine 加方法，它变成第二个 lib.rs；② `clip` 锁内若混入非纯库操作（如某域把文件 IO 塞进锁内方法），会拖住 vault/设置全部路径——该纪律目前只写在 stage 注释（mod.rs:342-344）里，无结构约束。

**建议**
- 止损线：SearchEngine 方法数不再增长——新域（若有）自带 facade，命令层直接调 `engine.stage_xxx()` 的模式改为 `engine.with_conn(|c| stage::xxx(c))` 风格或域 facade 挂在 SearchEngine 字段上（仿 `apps`/`recents`/`shortcuts` 三个具名句柄的既有先例，mod.rs:168-172）。
- 不建议现在拆 SearchEngine：现有方法都是薄转发，拆分收益低；仅需在 mod.rs:342 的锁纪律注释升级为模块级文档（谁能在锁内做什么）。

**置信度：高**。

---

### P2-9 命令层 55 个 spawn_blocking 样板：拆分前先消灭转换噪音

**证据**
- 典型形态重复 40+ 次：`let engine = state.engine.clone(); tauri::async_runtime::spawn_blocking(move || engine.xxx(..)).await.map_err(|e| e.to_string())?`（vault_search lib.rs:143-151、stage_list lib.rs:1250-1258 等几乎逐字相同）。
- 同步/异步的选择已有军规与例外清单（lib.rs:114-116 军规；save_settings 主线程理由 lib.rs:341-345；open_item 行号同步探存在 lib.rs:1158-1161）——例外都有注释，纪律良好。

**架构后果**
样板本身不是 bug，但它让 lib.rs 体积虚高（约占 40%），淹没真正的策略代码；拆分时机械搬运这些样板既费 review 精力又引入搬运错误风险。

**建议**
拆分第 4 步时顺带引入一个小 helper（如 `async fn on_pool<T>(f: impl FnOnce() -> Result<T,String> + Send + 'static) -> Result<T,String>`），命令体缩为一行。仅限命令层，不动 providers。可选项，不做也不影响拆分正确性。

**置信度：高**。

---

### P2-10 两处低频资源生命周期问题（记录在案，非紧迫）

**证据**
- `register_drop_target` 重试线程：快速 10 次 × 300ms 后转 **60s 永久低频重试**（stagedrop.rs:688-696），无退出条件（仅应用退出中 return）。窗口被裁掉/注册永久失败的场景下线程空转到进程结束；单线程 + 60s 一次 FindWindow 级调用，成本可忽略，但属于「无界生命周期的后台线程」模式。
- `paste_text_async`/`quick_paste_latest`/open_item 图片分支每次动作裸 `std::thread::spawn`（lib.rs:1095,1571,1194）+ 固定 80ms sleep。用户动作级频率，无风险；但三处各自重复「写剪贴板 + 等 80ms + paste_to_prev」的编排，语义重复。

**架构后果**
现状无害；前者在「1000 用户远程诊断」场景下会多出一条难解释的线程；后者三处编排若有朝一日改注入时序要改三处。

**建议**
- 前者：重试上限（如累计 10 分钟后停并落 diag 日志「drop target 注册放弃」）——一行改动 + 诊断闭环。
- 后者：拆分第 3 步（panel.rs）时把三处编排收成 `paste_text_and_focus(text, prev)` 单一入口（open_item 图片分支保留其解码差异即可）。
**置信度：高**。

---

## 3. 架构类流程更改建议（含拆分路线图）

### 3.1 lib.rs 五步拆分路线图（每步独立提交、行为不变、可独立回滚）

目标终态：lib.rs 只剩 `run()` + builder + setup 骨架 + `generate_handler!` 装配（约 300 行）；新增 `state.rs` / `settings.rs` / `panel.rs` / `commands/`（或单文件 commands.rs）。**顺序经过依赖方向设计：先解反向边（P1-5），再动无依赖的大块，策略层最后搬并当场提纯。**

| 步 | 内容 | 边界 | 风险 | 验证手段 |
|---|---|---|---|---|
| **0** | 解三条反向依赖边（P1-5）：handle_stage_drop 下沉、superpanel 动作改走中间层、lib.rs:90 改调 stagedrop API | 全部为等价改写，`crate::` 引用清零 | 低（stagedrop.rs:889 处注意 OLE 回调线程语义不变） | cargo test 全绿 + 真机拖放冒烟（两路径：自有 IDropTarget 与 wry 兜底） |
| **1** | 提取 `settings.rs`：AppSettingsDto / build_app_settings / save_settings / config_settings_from / transparency（lib.rs:225-664 整段搬移） | 与窗口逻辑零交集；save_settings 的 global_shortcut 调用随函数走；命令名/签名/线协议不变 | 低（纯搬移；Option 容错注释随行） | cargo test；设置页保存/热键冲突回滚/配置导出导入往返（appconfig 测试已锁协议） |
| **2** | 提取 `state.rs`：`AppState` + `DirectKind` + `spawn_hidden` | 纯类型搬移，无行为 | 近零 | cargo test + 编译 |
| **3** | 提取 `panel.rs`：hide_panel_window / toggle_panel / open_stage_panel / 前台捕获三函数 / paste_text_async / take_prev_foreground / quick_paste_latest / on_window_event 的失焦分支；`hold_panel`/`shown_at`/`prev_foreground` 三字段从 AppState 拆出为 `PanelState`（独立 `app.manage`）；**当场新增 `should_hide_on_blur` 纯函数（lib.rs:1707-1717 逐分支等价）+ 三事故场景单测** | AppState 余下 hotkey/direct_hotkeys/clip_item（设置域）由 settings.rs 消费；托盘/hotkey handler 改调 panel API | 中（事件接线 + 三窗口标签 main/superpanel/stagedrop 的分支归属）；本步是唯一「搬移 + 提纯」混合步 | 新增单测 + 手动回归三场景：呼出后点外部失焦即隐 / stage 视图 hold 期间失焦不隐 / 300ms grace 内抖动不隐 |
| **4** | 提取 `commands.rs`（或 commands/ 目录）：55 个命令适配器机械搬移，顺带做 P2-9 的 helper（可选） | 命令名一律不变，generate_handler 引用路径更新 | 近零（机械） | cargo test + tsc/vitest 全绿 + 面板全功能手测清单 |
| **5**（收尾） | lib.rs 瘦身后复查：run()/setup 里散落的设置装载（lib.rs:1788-1830）改调各 owner 模块（衔接 P0-2 阶段一） | setup 变为「调 owner 的 load 函数」序列 | 低 | 与 P0-2 阶段一同批验证 |

每步提交粒度 = 一个可 revert 的 commit；步骤间无交叉修改面（0 改 stagedrop/superpanel，1 改设置段，3 改窗口段），冲突面最小。

### 3.2 设置链路两阶段收口（承接 P0-2）

- **阶段一（Rust 内聚）**：以设置域为单位建 `load_from(conn)` / `apply(v)` / `persist(conn, v)` 三个 owner 函数（clipboard、stage、stagedrop、appearance、hotkeys 各自所属），save_settings 主体变为「校验热键 → 逐域 apply+persist → 托盘同步」；setup 变为「逐域 load」。镜像处 10→约 5，且新增设置从「记得改 9 处」变为「在 owner 模块加 3 行 + DTO/ConfigSettings 加字段（serde 派生）+ 设置页加控件」。
- **阶段二（TS 镜像消灭或锁定）**：优先轻方案——新增一条 vitest 契约测试（后端 DTO 序列化 JSON 的键集合硬编码进测试或经构建产物比对 SettingsView `AppSettings` 键集合）；若后续愿意引 specta/ts-rs 再升级为生成。**不动线协议本身**（camelCase 序列化与 ConfigSettings 兼容语义已有测试锁定，appconfig.rs:204-259）。

### 3.3 决策提纯清单扩展（继承 b523d22 纪律）

下述判定目前内联在 glue 里、无测试，按既有纪律提纯（落点随 3.1 各步）：
1. `should_hide_on_blur(hold, within_grace, fg_in_panel)` ← lib.rs:1707-1717；
2. `should_skip_paste_backfill(current_text, autopaste_on)` ← quick_paste_latest 的分流（lib.rs:1575-1590，「剪贴板已有文本直接注入 vs 回源历史」——该分支注释里全是产品语义，值得锁）；
3. open_item 的 `paste_started`/kind 收面板判定（lib.rs:1216-1226，含「stage 视图打开不收面板」拍板）提为 `should_close_panel(kind, paste_started) -> bool`。
三处都是纯函数、逐分支等价改写、各自补情形测试——与 decide/corner_origin/avoid_drag_origin 同一模式。

### 3.4 panic 痕迹进 diag（承接 P1-3）

`std::panic::set_hook` 挂在 run() 最前（diag::init 之后），钩子内 `diag::log` + 原有 stderr 输出。配合主线程 4 处 expect 改 poison-tolerant。提交粒度：一个 commit 两件事（都是错误路径，同批验证：人为 poison 一个锁验证不退出且 zap.log 有痕）。

### 3.5 锁序守卫（承接 P1-4 脆弱点一）

在 `direct_hotkeys` 与 `clip` 两个锁的获取点各加一行锁序注释（`direct_hotkeys → clip` 单向），并收紧 lib.rs:444 的锁作用域（register/DB 写移出临界区）。不引入锁序检查工具（单人维护成本不划算），用注释 + 作用域收紧把约束显性化。

---

## 4. 诚实清单

1. **全部结论来自静态阅读，未运行应用、未做动态竞态验证**。BAR_SHOWN 并入 STATE 的竞态分析、save_settings 锁作用域收紧后的行为等价，均需实施时以测试 + 真机冒烟兜底。
2. **测试计数口径**：按 `#[test]` 统计 Rust 共 120 个（16 文件），与简报「117 + 1 smoke」差 2 个，可能是统计时点或计数口径差异；不影响「lib.rs 零测试」这一核心论断。
3. **前端 SettingsView.tsx 只读了结构与头部**（接口定义、状态、备份流程开头），save 函数体与全部表单控件未逐行核对；「九处镜像」前端侧按「接口字段 + 表单控件 + 保存载荷」三处计，具体行号只给了接口处（SettingsView.tsx:69-100）。
4. **providers 未全文细读的模块**：apps.rs（36KB）、vault.rs（31.3KB）、winsys.rs、recent.rs、files.rs、clipimage.rs、dpapi.rs、sysutil.rs 只做了定向检索（锁、tauri 依赖、关键函数）；锁清单可能遗漏实例级（非 static）的锁。
5. **「无反向锁序边」结论基于 grep 级检查**（providers 无 AppState 引用），未做形式化验证；若有动态创建的间接回调路径（当前未见）则不成立。
6. **Tauri async 命令内 panic 的前端表现**（promise 永不 resolve、`.catch(() => {})` 静默吞掉）基于框架通识推断，未实测复现。
7. **round1 已有的 01-performance / 02-frontend / 06-product / 08-data 四份报告未读**——为保持本评审独立性未做交叉引用；若性能报告对 `clip` 单连接咽喉有不同结论，以其实测为准。
8. lib.rs:1169-1177（open_item 同步命令主线程做 DB 点查）与 lib.rs:934（shortcuts_list 同步命令）是军规的两个例外：前者有注释论证（lib.rs:1158-1161），后者经核实确为纯缓存读（shortcuts.rs:69-75 走 RwLock 缓存不碰库）——均不计为发现，仅在此备案。
9. vault.rs:110 在 Read 工具输出中呈现乱码，经字节级复核为**工具解码伪影**，文件本身 UTF-8 健康（与简报「磁盘内容健康」一致），不是 GBK 污染。
