# 11 · 红队技术审查（round2 · 对抗性复核）· 2026-10-29

评审人：红队技术审查员。任务 = 对 round1 全部技术类 P0/P1 发现做对抗性复核：亲验证据、查在途改动覆盖、重估严重度、裁决八项跨报告矛盾。
事实源 = 磁盘当前状态。工作区实况（`git status`）：**源码零未提交改动**（简报所述 5 文件在途改动已以 ad99093 落库），仅 `reviews/` 未跟踪。所有行号以当前磁盘为准；tauri-plugin-updater 行号以本机 registry 的 **2.14.0**（Cargo.lock:4639-4640 锁定版本）为准。

---

## 1. 裁决总表

只列技术类 P0/P1（P2 证据经抽查无一错报，全部维持，不逐条列表；行号勘误集中在 §4 诚实清单）。统计：**维持 22 · 降级 4 · 驳回 1 · 改写 3**。

| 原报告·原级别 | 裁决·新级别 | 一句理由（证据见 §2/§3） |
|---|---|---|
| R1·P1-1 超级面板恢复走 arboard set_image | **维持 P1** | superpanel.rs:96-107 set_image 恢复先于 :111 show_panel，项目自测注释 clipboard.rs:47-53 背书 39MB→5-6s；DIBV5 直写 clipboard.rs:54 现成 |
| R1·P1-3 open_item 主线程全文本点查+SQLite 写 | **维持 P1** | lib.rs:1108 同步命令、:1171-1176 全文点查、:1115-1119 主线程写均属实；ad99093 只把全文移出 IPC，点查仍在主线程（commit 有意为之）；「+一次 WAL fsync」对 clip 分支表述不准（读不 fsync），见 §2-④ |
| R2·F1 右键菜单键盘面整体失效 | **维持 P1** | onKeyDown 绑输入框（App.tsx:1239）、行无 tabIndex（:1285）、菜单分支在 onKeyDown 内（:1036-1042）、窗口捕获被 mergeMode 门死（:813）——右键后焦点掉 body，全链闭合 |
| R2·F2 合并键盘面泄漏进设置页/中转站 | **维持 P1** | openStage(:328-331)/openSettings(:454-459)/preset(:341-348) 均不清 mergeMode；唯一退出是 hidden(:274) 与 vault 守卫(:798-803)；捕获对 Enter/Esc 无条件拦截（:825-846） |
| R2·F3 useEscCapture 缺 isComposing 卫兵 | **维持 P1** | hooks/useEscCapture.ts:11-16 确无卫兵；团队在 App.tsx:815/StageView.tsx:417 自加卫兵并注明用途，是最强旁证 |
| R3·P1-1 Everything 无认证 HTTP 服务 | **维持 P1，加重** | everything.rs:162-166 三行配置、files.rs:42/89-92 零认证查询均属实；红队新发现-1：用户手动关掉 HTTP 会被 seed_ini 静默改回（§3） |
| R3·P1-2 浏览器 PM 密码入历史 | **并入 R8·P1-1，维持 P1** | 与 R8 是同一根因的两面，合并为一条（见 §2-⑦） |
| R3·P1-3 备注/用户名/超级面板走普通剪贴板通道 | **维持 P1** | vault.rs:281(username)/:322(notes) 走 copy_text，密码/TOTP 走 copy_text_private(:295/:368)；vault.rs:315 注释自称「抑制剪贴板历史」与 begin_self_copy 实际语义（只压 zap 自身入库）不符，加重 |
| R4·P0-1 lib.rs 巨石无测试 | **降级 P1** | 事实全对（2024 行、lib.rs 零测试、失焦策略无锁），但它不是运行时事故而是演进风险——按「生产环境真实后果」口径不到 P0 |
| R4·P0-2 设置九处镜像 | **降级 P1** | 九处逐一抽查属实（appconfig.rs:40-41/100、stagedrop.rs:45、lib.rs:1820-1824/247/288/364+530-536/608、SettingsView.tsx:88），「漏一处即静默失效」是未来概率非现行事故 |
| R4·P1-3 Mutex poisoning 主线程 panic | **改写·降 P2** | **R5 胜**：Cargo.toml:62 `panic="abort"` 属实，abort 下无 unwind → std Mutex 永不 poison → 15 处 expect 在 release 是死代码；panic hook 部分保留并入 R5 P0-1 修复（见 §2-①） |
| R4·P1-4 嵌套锁无守卫 | **维持 P1** | lib.rs:444 持 direct_hotkeys 锁期间 :446-451 get_setting、:456/:473 set_setting——`direct_hotkeys → clip` 嵌套亲验属实 |
| R4·P1-5 providers 反向依赖 | **维持 P1** | superpanel.rs:279 `crate::toggle_panel`、lib.rs:90 直写 STAGE_VIEW_OPEN 亲验；stagedrop.rs:889 与 R4 引用一致 |
| R4·P1-6 App.tsx vault 域逻辑栖身根组件 | **维持 P1** | 结构债与 lib.rs 同病，级别恰当（P1 而非 P0） |
| R5·P0-1 任意 panic = 进程静默死亡（abort×无 hook×stderr 不可见） | **维持 P0** | Cargo.toml:62、main.rs:2、全仓无 set_hook（findstr 零命中）、mod.rs:179 expect 在 setup 内（lib.rs:1785）——链路全部亲验 |
| R5·P1-1 watcher 三处静默死亡 | **维持 P1** | clipboard.rs:247/257/317/325/328 全部只 eprintln，release 下丢弃，无 respawn——逐行属实 |
| R5·P1-2 无 busy_timeout → 静默丢条目 | **驳回** | **R8 胜**：rusqlite 0.40.2 inner_connection.rs:118 默认 `sqlite3_busy_timeout(db, 5000)`——「默认 0ms 后到者立即 SQLITE_BUSY」前提错误；残余面（主线程写最长等 5s）已被 R8 P1-3 覆盖，并入（见 §2 附注） |
| R5·P1-3 关键失败走 stderr | **维持 P1** | lib.rs:1854/1884/1895、clipboard.rs 五处、files.rs:94-106 逐点属实 |
| R5·P1-4 超级面板失败分支跳过剪贴板恢复 | **维持 P1** | superpanel.rs:84-89 `else { return }` 确在 :91-107 恢复逻辑之前——seq 已变但 get_text 失败即不恢复，亲验属实 |
| R5·P1-5 DB 损坏无恢复路径 | **改写·并入矛盾⑧合并 P0** | 与 R8 P0-2 同一链路，合并定级（§2-⑧） |
| R7·P0-1 工具空态教按 Ctrl+V 但监听未挂载 | **降级 P1** | 机制全证实（panel.tsx:53、JsonWorkbench.tsx:211、ToolWorkbench.tsx:335、App.tsx:1168-1170 传 text=""），但同句给了可用的鼠标出路，2 秒可恢复、零数据后果——不到 P0（§2-⑥） |
| R8·P0-1 文本 clip 无上限 | **降级 P1，机制维持** | 入库无上限（clipboard.rs:280-284）、top-300 逐行 replace 现算（:360-363）属实；但「打字卡死/应用不可用」夸大——搜索在 spawn_blocking，输入框不卡；真实后果=结果延迟+锁竞争放大+DB 永久高水位+无自救（§2-②） |
| R8·P0-2 库损坏=启动 panic | **维持 P0，与 R5 P0-1/P1-5 合并** | mod.rs:179、vault 同库（mod.rs:180-183）亲验；合并定级见 §2-⑧ |
| R8·P1-1 密码明文入库且无单条删除 | **维持 P1（含 R3 P1-2 合并）** | invoke_handler 清单 lib.rs:1722-1778 亲验：只有 clip_pin/clip_row/clip_merge_paste，无任何删除/清空命令 |
| R8·P1-2 无 user_version 迁移机制 | **维持 P1** | clipboard.rs:212-215、vault.rs:32-35 吞错 ALTER、clipimage.rs:24 吞错建表亲验属实 |
| R8·P1-3 写竞争兜底/主线程写冻结 | **维持 P1** | rusqlite 默认 5000ms（inner_connection.rs:118）亲验——R8 判断正确；save_settings 十余次独立 autocommit（lib.rs:497-568 无事务）与 record_app_launch 主线程写（lib.rs:1115-1119→apps.rs:294-300）均属实 |
| R9·P0-1 Windows 更新退出且不自动重启 | **改写·降 P2** | **裁决：R9 引文一半对一半错**——`process::exit(0)` 属实（updater.rs:1050，ready 态确实不可达），但插件默认 `restart_after_install=true`（updater.rs:226）+ 默认 Passive 模式追加 `/R`（config.rs:21-22/53-57）→ NSIS 装完自动拉起；「更新把 zap 搞没了」不成立（§2-⑤） |
| R9·P1-1 无签名×高启发式特征 | **维持 P1** | SendInput（winsys.rs:201/superpanel.rs:157）、DPAPI（dpapi.rs:34/76）、剪贴板常驻、HKCU 自启（lib.rs:1998-2004）、下载提权（everything.rs:82/254-268）逐一实证 |
| R9·P1-2 更新检查仅启动一次 | **维持 P1** | useUpdater.ts:49-58 单次 useEffect 无定时器；托盘菜单 lib.rs:1928-1959 无检查更新项 |
| R9·P1-3 崩溃诊断三缺 | **维持 P1** | 与 R5 P0-1 的 panic hook 修复是同一件事，合并实施 |

不裁项：R6·P0-1（分发信任断层）与 R6·P0-2（首启引导缺失）属产品维度判断，其技术成分（未签名、热键失败仅 eprintln）已由 R9 P1-1 / R6 P1-5 覆盖，级别不做技术裁决。

---

## 2. 八个矛盾点的实证裁决

### 矛盾① poisoning（R4 P1-3）vs panic=abort（R5）→ **R5 论证正确，R4 P1-3 改写降 P2**

- 代码证据：`src-tauri/Cargo.toml:62` 确有 `panic = "abort"`（R5 没引错）；`main.rs:2` windows_subsystem 属实；全仓 `findstr set_hook panic::` 零命中。
- Rust 语义：std `Mutex` 只在**持锁线程 unwind 时**置 poison 标志。abort 语义下首个 panic 直接走 abort 路径，不展开栈 → 任何线程都观察不到 poisoned 状态 → `lib.rs:1473-1476/1493/1710-1713/1647/1658`（全部亲验存在）的 `expect("... poisoned")` 分支在 **release 构建中不可达**，R4 描述的「此后每次呼出/失焦都重复 panic」在 release 下不可能发生（首 panic 即一次性 abort）。
- 保留价值：R4 建议中的 panic hook（写 diag）与 R5 P0-1 建议 1 完全重合，价值成立；poison-tolerant 改写在 abort 下属死代码风格统一，降为 P2 顺手项。
- 结论：矛盾双方中 R5 对「可达性」的判断准确；R4 对「一旦发生后果严重」的论证建立在 unwind 前提上，前提不成立。

### 矛盾② 文本 clip 膨胀与卡顿（R8 P0-1）→ **机制属实、量级改写、降级 P1**

- 代码证据：入库零长度检查（clipboard.rs:276-284 `INSERT ... VALUES(?1,?2)` 全量直插）；搜索 SQL `substr(content,1,256)` 与 `length(content)-length(replace(content,char(10),''))` 对 top-300 逐行现算（clipboard.rs:357-366）；空查询 `latest()` 拖最新一条全文（clipboard.rs:539-546 → mod.rs:692-699）。
- 简报所说「评分只吃前 256 字符且 SQL 侧 substr 截断」只对**打分与列表**成立（clipboard.rs:354-355 注释自认），**不挡入库也不挡换行数计算**——R8 没有误读。
- 但后果措辞需要改写：search 全程在 `spawn_blocking`（lib.rs:64-74），**主线程与输入框不被卡**，用户感知是结果列表刷新变慢，不是「打字卡死」；「应用不可用」不成立。真实的放大链在锁上（红队新发现-3）：巨型行进 top-300 后，每次击键的 needs_conn 插件在 clip 锁内做完扫描（mod.rs:654-661），主线程 `open_item` 的 clip_row / `record_app_launch` / `save_settings` 抢同一把锁（lib.rs:1171-1176、:1115-1119、mod.rs:330-333）→ 周期性主线程卡顿。
- 定级：触发前提罕见（数百 MB 文本进剪贴板），后果可恢复（等 300 次新复制自然淘汰/手删库），修复廉价（入库上限一行）→ **P1**，与「无单条删除入口」（R8 P1-1）作为补救面合并推进。

### 矛盾③ 图片 GB 级（R1 P1-2）vs 膨胀主风险在文本（R8）→ **两边各对一半，R1 维持 P1、R8 降 P1，修复合并**

- 定量（全部亲验）：图片单条上限 16MB（clipimage.rs:16，store 超限拒收 :115-117）；图片表与文本表**各自独立**适用 RETENTION_LIMIT=500（clipboard.rs:286-291 文本、clipimage.rs:139-144 图片）→ 图片理论上限 500×16MB=8GB；现实 4K 截图 PNG 2-6MB → 重度截图用户稳态 **1-3GB 渐进积累**。文本单条无上限 → **单事件**即可抬升数百 MB 且永不回落（无 VACUUM/checkpoint，全库唯一 PRAGMA = clipboard.rs:205，grep 证实）。
- 谁先出事：**磁盘稳态看图片**（每周都涨、可预期到 GB 级），**尖峰与击键成本看文本**（一次事件、无法自救）。两条不是同一风险的两面，是同一修复单的两个输入：图片按字节预算淘汰 + 文本入库上限 + 退出 checkpoint/低频 VACUUM。R1 P1-2 维持 P1，R8 P0-1 降 P1，建议在同一 PR 落地。

### 矛盾④ open_item 主线程点查是否已被在途改动修掉 → **没有，且是有意保留；R1 P1-3 维持**

- 代码证据：ad99093 提交信息明示「open_item clip 分支按行号回查全文回贴（**主线程 PK 点查与 clipimg exists 同模式**）」；磁盘现状 lib.rs:1171-1176 `engine.clip_row(rowid)`（全文 SELECT，mod.rs:271-274 在 clip 锁内执行）仍在同步命令 open_item（lib.rs:1108，主线程）里；注释 lib.rs:1161-1162 自述理由：「已被清理」要同步回 toast，不能异步化。
- 判定：在途改动改变的是**数据源**（IPC 携带全文 → 按行号回查），没有把点查移出主线程。R1 P1-3 证据属实、定级维持。两处修正：① 「+一次 WAL fsync」贴错了分支——clip 文本分支是纯读（WAL 读不 fsync），fsync 属于同函数 app 分支的 record_app_launch（apps.rs:295-299 的 upsert 写）；② 图片分支的两段式模式（lib.rs:1177-1205）确实是文本分支应有的对齐目标，R1 修法成立。

### 矛盾⑤ Windows 更新退出且不自动重启（R9 P0-1）→ **前端事实对、核心断言错，改写降 P2**

- 代码证据（本机 cargo registry，tauri-plugin-updater **2.14.0** = Cargo.lock 锁定版）：
  - `updater.rs:1050`：spawn 安装器后 `std::process::exit(0)` → **应用确实被退出**，`downloadAndInstall` 的 await 永不 resolve → `setUpdateStatus("ready")`（useUpdater.ts:108）与「重启以完成更新」按钮（SettingsView.tsx:1246-1250、1285-1294）在 Windows 上确实不可达——R9 的前端侧论证全部成立；
  - **但** `updater.rs:226`：`restart_after_install: true` 是构造默认值（zap 的接线 lib.rs:1683 用裸 `Builder::new().build()` 未覆盖）；默认安装模式 Passive（config.rs:21-22 `#[default]`）；Passive 下 `nsis_restart_after_install_args()` 返回 `["/R"]`（config.rs:53-57，注释明写 `/R: Restart`）；updater.rs:1064-1078 将其与 `/UPDATE`、`/ARGS <当前 exe 参数>` 一起下发给 NSIS。
- 结论：装完**自动重启**。用户真实体验 = 进度到 99% → zap 消失 → 数秒后 zap 以新版回到托盘。「更新把 zap 搞没了」的 P0 叙事不成立。残留真问题（P2）：① 无「更新完成」正反馈，ready 态是死 UI；② 消失→重启之间的空窗无解释；③ downloadAndInstall 的 catch 分支会把安装器启动失败误报为「下载更新失败」。修法从 NSIS hook 自动拉起改为：ready 分支文案改为「更新将自动完成，zap 即将重启」+ 补完成反馈。
- 保留 R9 的另一半：P1-2（启动检查一次）维持 P1——那个是独立且全对的。

### 矛盾⑥ 空态教 Ctrl+V 但监听未挂载（R7 P0-1）→ **机制全对、降级 P1**

- 代码证据链：粘贴监听仅在 `awaitPaste===true` 时挂载（panel.tsx:52-53 `if (!awaitPaste) return;`）；`startAwait` 只由「粘贴替换」按钮触发（JsonWorkbench.tsx:156-161）；入口词进入传 `{kind:"empty"}`（App.tsx:855-868）→ App.tsx:1168-1170 对 empty 源显式传 `text=""` → 空态必然显示「按 Ctrl+V 粘贴，或点右上『粘贴替换』」（JsonWorkbench.tsx:211、ToolWorkbench.tsx:335）；App.tsx 无全局 paste 监听兜底（findstr 零命中）→ 第一次按 Ctrl+V 确实无任何反应。新用户必踩，R7 事实 100% 成立。
- 定级分歧：空态**同一句话里就给了可用的鼠标出路**（点右上按钮），2 秒恢复、无数据后果、非破坏性。本项目 P0 口径（R5/R8/R9 的 P0 均为「进程死/数据丢/无法自救」级）容不下这条 → **降 P1**。修法照 R7：监听常驻（panel.tsx 注释自证非可编辑焦点下 paste 可达），零新键。

### 矛盾⑦ 浏览器 PM 密码入历史（R3 P1-2）vs 手动密码无删除（R8 P1-1）→ **合并为一条 P1**

- 代码证据：过滤名单只认 8 个桌面 PM 进程名片段（clipboard.rs:191-200），非命中即全文入库（:276-284），标题回显搜索列表（:460-467）；invoke_handler 全清单无删除/清空命令（lib.rs:1722-1778 亲验）。
- 裁决：**同一根因的两个症状**（「会进来」与「出不去」），合并为一条 P1：*非名单来源的密码明文入库、驻留至多 500 条滚动窗口、无任何用户补救动作*。两份报告的修法互补不重复：R3 的入库前与 vault 密码集比对 = 预防；R8 的 clip_delete/clip_clear_unpinned + 右键入口 = 补救——建议一个 PR 双管齐下，与拍板「便捷优先」不冲突（右键不新增组合键）。定级维持 P1：单用户本地威胁模型下（R3 §1 自立的前提）不是 P0，但 1000 用户规模下必然发生且零补救，够 P1。

### 矛盾⑧ DB 损坏 panic（R5 P0-1 链 / R8 P0-2）→ **合并为一条 P0，两个成分都是真 P0**

- 代码证据：mod.rs:179 `clipboard::open_db(db_path).expect("打开剪贴板数据库失败")`（亲验）；它在 setup 闭包内执行（lib.rs:1785）；vault/设置/stage/stats 六张表全部建在同一连接同一库（mod.rs:180-183 亲验）；abort + windows_subsystem + 无 panic hook → 损坏即「双击无反应、无对话框、zap.log 无痕」。
- 两个成分分开评：① **启动失败静默无日志**——真 P0 成分（不可诊断，远程排障为零线索）；② **唯一出路手删库连 vault 一起丢**——真 P0 成分（不可恢复数据丢失，vault 仅手动导出可救）。二者不分高下，合并条维持 **P0**。
- 修法收敛（R5/R8 建议一致，红队背书）：panic hook（所有 P0/P1 链的公共底座）→ open_db 失败/quick_check 失败改名为 `zap.db.broken-<ts>` 换新库 + diag 记录 → 前端提示「账号库可从 .zapvault 恢复」。R5 P1-5 的「上一次导出时间 + 30 天未导出提醒」并入。

---

## 3. 红队新发现（全部角色漏掉，均经本人代码验证）

### 新-1（P2，R3 P1-1 的放大器）zap 会静默复活用户手动关闭的 Everything HTTP 服务

- 证据：`everything.rs:316-331 ensure()` 在 probe 失败且已安装时调 `setup()`；`setup()`（everything.rs:282-303）在 :295 无条件调 `seed_ini()`；`seed_ini` 的 FIXED 四行含 `http_server_enabled=1`（:162-166），对已有 ini 按**键名前缀替换**（:188-194）——用户在 Everything 里手动把 HTTP 服务关掉（`http_server_enabled=0`），下一次面板呼出触发的自动启动尝试（everything_autostart 开启时，最多每 5 分钟一轮）就会把它**静默改回 1**。
- 后果：R3 P1-1（无认证全盘索引 HTTP 面）对用户而言「关不掉」——唯一的自救动作被 zap 撤销，且无任何提示。与 R3 P1-1 的修复（加认证）合并处理；若暂不加认证，至少 seed_ini 应尊重既有值（只补缺、不覆盖 0）。
- 置信度：高（ensure→setup→seed_ini 链与替换逻辑逐行亲验）。

### 新-2（P2）超级面板动作识别对全文裸跑 serde_json，无 256KB 卫兵

- 证据：`superpanel.rs:186` `serde_json::from_str::<serde_json::Value>(t)` 直接吃热键抓取的选文全文。同链路所有其它识别都有上限：texttools `recognize`/`convert_all` 的 `MAX_INPUT_BYTES`（texttools.rs:138、:277-283）、jsontools 同款——唯独这一处没有。
- 后果：在编辑器里选中数 MB 文本按超级面板直达热键 → `build_actions` 在 `show_panel`（superpanel.rs:220-244）之前同步解析，非 JSON 时逐字符扫描、JSON 时全量建 Value——热键→菜单延迟秒级 + 2-3× 内存峰值；与 R1 P1-1（图片恢复路径延迟）同型但入口不同。修法一行：解析前判 `t.len() > MAX_INPUT_BYTES` 跳过 JSON 识别。
- 置信度：高（路径确定；量级为大字符串解析的常规推断）。

### 新-3（P1，矛盾②的锁面补充）clip 单咽喉锁无持有时长预算：巨型内容会把搜索成本转嫁为主线程冻结

- 证据：空查询组装在 clip 锁内完成 `latest()` 全文取 + top-300 扫描 + 全插件 detect（mod.rs:495-503，锁跨整个 `assemble_opening`）；每次击键 needs_conn 插件在同一把锁内串行执行 search（mod.rs:654-661）；主线程同步命令 `open_item`（lib.rs:1169-1176 clip_row / :1115-1119 record_app_launch）与 `save_settings`（mod.rs:260-263 set_setting 走同一把锁）都要抢它。
- 后果：一条巨型 clip 进入 top-300 后，**每次击键把锁持有时间拉长到扫描时长**，主线程的启动应用/保存设置/回贴全部排队——这是 R8 P0-1 的「卡顿」真正能被用户感知为「zap 卡」的通路，round1 十份报告无人把它与锁拓扑串起来（R1 诚实清单只写了「conn 类持锁时长未测」）。
- 处置：不单独立项改架构（单咽喉本身是 R4 P2-8 认可的成本模型），作为 R8 P0-1（降 P1 后）的加重证据与修复动机——入库上限 + 换行数物化为列（消灭每击键 replace 扫描）顺带把锁持有时长压回毫秒级。
- 置信度：高（锁拓扑逐点亲验；卡顿时长依赖巨型行存在，触发前提与 R8 P0-1 相同）。

---

## 4. 诚实清单

- **全程静态阅读 + 本机 cargo registry 源码核验，未运行应用**。registry 证据：rusqlite-0.40.2（inner_connection.rs:118）、tauri-plugin-updater-2.14.0（updater.rs/config.rs，版本经 Cargo.lock:4638-4640 确认）。
- **NSIS 模板 `/R` 的实现体未亲见**：模板在构建侧 tauri-bundler（本机 registry 无该 crate）。我依据的是插件源公开契约（updater.rs:413「default is `true`」、config.rs:44 注释「/R: Restart」与 :53-57 的参数表）——这是插件作者承诺的行为，但「装完自动重启」的最终确认仍需一次真机发版验证（与 R9 诚实清单同项，方向相反：现在要验证的是「确实会重启」而非「确实不会」）。
- **Rust 测试计数三处口径打架**（简报 117+1 / R4 数 120 / 最新提交 69b3c5b 自述 119+1），未实跑 `cargo test` 裁决；vitest 盘面 31 vs 简报 32 同。不影响任何裁决。
- **继承自 round1 的未真机项**：Chrome 复制密码时剪贴板 owner 进程归属（矛盾⑦触发前提）；IME 合成期 Esc 的 key 形态（R2 F3，团队卫兵先例为旁证）；arboard set_image 5-6s 量级引自项目自测注释（clipboard.rs:47-50）；Defender 误报概率（R9 P1-1）。
- **行号勘误（不影响结论）**：R8 引 `latest()` 为 clipboard.rs:519-526，实为 :539-546；R8 引 mod.rs:692-699 检测路径、:179、:640-668 均准确。R1 引 open_item 为 lib.rs:1107，fn 行为 :1108（#[tauri::command] 在 1107），准确。
- **未逐行通读**：apps.rs（定向核验 record_launch/init_stats 段）、stage.rs（定向核验 finish/锁段）、stagedrop.rs、backup.rs、jsontools.rs 全文、stagedrop.rs:889 反向依赖仅凭 R4 引用与同型 superpanel.rs:279 亲验类推。
- **无法裁决项**：R6 两条 P0 的产品定级（§1 已注明不裁）；R3 P1-2 的「Chrome owner=chrome.exe」前提（列于上）。
- 本轮只读，未修改任何源文件；未生成临时文件。
