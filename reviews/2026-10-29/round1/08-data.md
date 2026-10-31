# 08 · 数据工程师视角（数据与存储维度）· 2026-10-29

评审事实源：磁盘当前状态（含未提交在途改动）。评审范围：schema 全景、剪贴板数据生命周期、多连接 WAL 拓扑、备份、配置文件、数据完整性、敏感数据留存、数据搬家。全部论断经源码核验，路径行号以当前磁盘为准。

---

## 1. 角色结论摘要

1. **单库三连接的 WAL 拓扑方向正确**：引擎共享连接（Mutex）+ 监听线程写连接 + 图片 worker 连接并存（`providers/mod.rs:163-165`、`providers/clipboard.rs:243-254`），WAL 下读不阻写；但**写-写冲突面完全依赖 rusqlite 默认 5s busy_timeout 兜底**（rusqlite 0.40.2 bundled，`Cargo.toml:30`；源码 open 路径设 `sqlite3_busy_timeout(db, 5000)`），主线程同步命令里的写操作（`save_settings`、`open_item` 的 `record_app_launch`）可被图片 worker 的 16MB BLOB 大事务阻塞。
2. **迁移机制不存在**：无 `PRAGMA user_version`，演进靠 `CREATE TABLE IF NOT EXISTS` + 吞错 `ALTER TABLE`（`clipboard.rs:212-215`、`vault.rs:32-35`）。加列能活（现在就是这样加的 pinned/totp_enc），破坏性变更（改类型/约束/回填数据）没有任何路径。
3. **膨胀主风险不在图片在文本**：图片有 16MB/条上限且保留条数明确（`clipimage.rs:16`、`clipboard.rs:188`，共享 RETENTION_LIMIT）；**文本 clip 单条无任何大小上限**（`clipboard.rs:280-284` 直插全量），且搜索 SQL 对 top-300 行逐条算 `length(replace(content,...))` 换行数（`clipboard.rs:358-362`）——一条巨型 clip 每次击键都被全文扫描，且用户**没有任何单条删除/清空历史的入口**，只能等保留条数自然淘汰。
4. **事务纪律参差但关键路径已收口**：vault 导入（`vault.rs:470-494`）、备份恢复（`vault.rs:573-615`）、stage 批量恢复/批量移出（`stage.rs:377-389`、`stage.rs:396-411`）均整批一个事务（简报提到的 restore 事务**核实属实**）；但 stage 批量放入（`stage.rs:94-114`）与 `save_settings` 的十几连写（`lib.rs:497-568`）无事务，与 remove_batch 注释里自己论证过的成本模型不一致。BLOB 与行同体存储，无孤立 BLOB 问题。
5. **可救性薄弱**：zap.db 无任何自动备份/损坏自愈，启动时打开失败直接 `expect` panic（`mod.rs:179`）；vault 数据唯一救生通道是用户手动导出的 .zapvault；NSIS 卸载不清理数据目录（`tauri.conf.json` bundle 无 deleteAppDataOnUninstall），残留含**明文剪贴板历史**。本机实测：`%APPDATA%\com.ais.zap\zap.db` 34MB + WAL 5.9MB + icons/ 149 文件（prune 正常工作，`apps.rs:171-186`）。

---

## 2. 发现清单

### P0-1 文本 clip 无容量上限：单条巨型文本可拖垮整库与每次击键搜索

- **证据**：监听线程对文本不做任何长度检查直接 `INSERT INTO clipboard_history(content,...)`（`providers/clipboard.rs:276-284`），`content TEXT` 无 CHECK/上限（`clipboard.rs:206-210`）。搜索 SQL 每次调用对 top-300 行现算 `length(content) - length(replace(content, char(10), ''))`（`clipboard.rs:358-362`）——`replace` 会在 SQLite 内**分配一条与原文等大的新字符串**。呼出面板时空查询还把最新一条的**全文**拖出（`clipboard.rs:519-526` 的 `latest()` → `mod.rs:692-699` 喂给 detect）。识别/工具侧有 256KB 上限（`jsontools.rs:12`、`texttools.rs:18`）但那只挡识别，不挡入库与 SQL。
- **生产后果**：用户复制一份大日志/base64（数百 MB 在 Windows 剪贴板完全可行）→ 该条进 top-300 窗口后，**每次击键 = 两次全文级内存分配与扫描**，面板打字卡死；同时 WAL+DB 高水位一次性抬升数百 MB（见 P2-8，永不回落）。淘汰该条需要它跌出 top-300——默认 500 条保留下可能要数百次复制，**期间用户无任何补救手段**（无单条删除，见 P1-2）。触发不常见，但一旦发生即「应用不可用且无法自救」。
- **建议**：入库前设硬上限（如 1MB，超限截断或拒收——拒收更诚实，截断会悄悄丢数据语义）；同时把换行数与首行在**入库时**物化为列（`nl_count`、`head`），搜索 SQL 改读物化列，顺带消灭每击键的 O(n) 计算。权衡：需要一次迁移（见 P1-3 的迁移机制）；超限拒收要在文档/UI 说明「超长文本不入历史」。
- **置信度**：高（机制逐行核验）；实际触发频率中等偏低。

### P0-2 库损坏 = 启动即 panic，无自愈、无 DB 级备份，vault 只剩手动导出可救

- **证据**：`SearchEngine::new` 里 `clipboard::open_db(db_path).expect("打开剪贴板数据库失败")`（`providers/mod.rs:179`）——open_db 的 `execute_batch`（建表）在库文件损坏时返回 Err → setup 闭包 panic → 应用无法启动。全库无 `integrity_check`、无 VACUUM/checkpoint、无损坏转移逻辑（全库 PRAGMA 仅 `journal_mode=WAL`，`clipboard.rs:205`）。备份只有 vault 的**手动**加密导出（`lib.rs:965-974`；`backup.rs` 全文仅覆盖账号条目），clip/stage/设置无任何备份通道。
- **生产后果**：磁盘故障/杀软干扰/断电写坏 zap.db → 应用启动即死且**无诊断输出**（GUI 无 stderr），用户唯一出路是手工删库——剪贴板历史、中转站、启动计数全灭；vault 若从未导出过 .zapvault 则一并丢失。这正是「损坏不可恢复」级别：损坏概率低（synchronous=FULL 默认 + WAL），但一旦发生没有任何软着陆。
- **建议**：① 启动打开失败/`quick_check` 失败时把坏库改名 `zap.db.broken-<时间戳>` 换新库启动，`diag::log` 记录并提示用户；② 退出时 `PRAGMA wal_checkpoint(TRUNCATE)`（顺带收 WAL，见 P2-8）；③ 可选：每日首启把 accounts 表（DPAPI 密文态）自动快照一份到 `backup-auto/`（保留 N 份），让 vault 摆脱「用户忘记导出 = 裸奔」。权衡：换新库会丢数据，但比 panic 循环好且坏库保留可手工抢救；自动快照是密文态、无新增泄露面。
- **置信度**：高。

### P1-1 手动复制的密码明文入库，且无单条删除/清空入口——隐私不可补救

- **证据**：密码管理器过滤只认 8 个进程名路径片段（`clipboard.rs:191-200`：keepass/bitwarden/1password/enpass/dashlane/lastpass/roboform/keyguard），命中才不入库（`clipboard.rs:278`）。**从浏览器密码框、聊天记录、记事本手动复制的密码，owner 进程不在名单里，明文进 `clipboard_history`**。vault 自己的复制走 `begin_self_copy()` 抑制窗 + `copy_text_private`（排除 Win+V/云剪贴板，`vault.rs:59-71、280-297`）只保护 vault 来源。遍历 invoke handler 全部命令（`lib.rs:1722-1777`）：**没有任何删除单条 clip 或清空剪贴板历史的命令**（stage 有 `stage_clear`/`stage_remove_batch`，clip 只有 `clip_pin`）。DB 明文落盘（无 SQLCipher；`tauri.conf.json` 无加密配置）。
- **生产后果**：用户某天从网页复制了一次密码 → 该密码以明文躺进 zap.db 直至被保留条数淘汰（默认 500 条后），且用户发现后**无法手动删除该条**，只能整库手删（连 vault 一起丢）。1000 用户规模下这是必然发生的隐私事故，且无补救动作可给。
- **建议**：最小而尊重既有拍板的修法 = 给 clip 补上「单条删除 + 清空未固定历史」两个命令与右键入口（复用 stage 的删除交互心智，进入抑制窗防自收录）；进阶可选 = 复制内容含「密码/password/密码：」类标记行时提示不入库（误报率高，谨慎）。权衡：右键菜单不加新组合键，符合交互拍板；过滤启发式建议不做。
- **置信度**：高（过滤范围与命令清单均已逐一核验）。

### P1-2 迁移机制缺失：无 user_version，吞错 ALTER 模式只覆盖「加列」一种演进

- **证据**：建表全部是 `CREATE TABLE IF NOT EXISTS` + `let _ = conn.execute_batch("ALTER TABLE ... ADD COLUMN ...")` 吞错（`clipboard.rs:212-215`、`vault.rs:32-35`）；三张表（clipboard_history/clipboard_images/accounts）+ app_settings + stage_items + app_stats 六张表均无版本标记。init 失败一律 `let _ =` 静默（`clipimage.rs:24`、`appconfig.rs:51`、`apps.rs:277`）。三连接各自在 open_db 里重复跑同一套建表/ALTER（`clipboard.rs:243-254` 两个线程各开连接）。
- **生产后果**：当前模式加列尚可（错误吞掉即幂等），但 ① 破坏性变更（改约束/类型/拆表/数据回填）没有执行点；② 任何一次 ALTER 因 SQLITE_BUSY 等原因失败会被静默吞掉，后续引用该列的 SQL prepare 失败 → `search()` 返回空列表级别的**静默降级**（`clipboard.rs:358-364` 的 `else { return Vec::new() }`）；③ 无法远程诊断「用户库里 schema 是哪个版本」。
- **建议**：在 `open_db` 单点引入 `PRAGMA user_version` 迁移器：`BEGIN IMMEDIATE` → 事务内重读 user_version → 按 `if v < N` 逐步执行 → 写回版本号 → commit。IMMEDIATE 让三连接并发跑迁移时后者排队并在事务内重读到新版本，天然幂等；每个迁移步骤写成具名函数便于测试。权衡：约 50 行一次性投入；现有吞错 ALTER 收编为 version 1/2 步骤后即可删除。
- **置信度**：高。

### P1-3 写-写竞争靠默认 5s busy_timeout 兜底；主线程同步命令持写可被大图事务冻结

- **证据**：三个连接均为 `Connection::open` 默认参数，未显式设 `busy_timeout`/`synchronous`（`clipboard.rs:202-218`）；rusqlite 0.40.2 open 路径默认 `sqlite3_busy_timeout(db, 5000)`（registry 源 `inner_connection.rs`，已核验）。写者三方：监听线程每次复制 INSERT+DELETE（`clipboard.rs:280-291`）、图片 worker 最多 16MB BLOB INSERT+DELETE（`clipimage.rs:126-139`）、引擎连接（vault/设置/stage/pin/stats）。其中 `save_settings` 是**刻意留在主线程的同步命令**（`lib.rs:341-347` 注释），内部十余次 `set_setting` 独立 autocommit 写（`lib.rs:497-568`）；`open_item` 在主线程 `record_app_launch` 写库（`lib.rs:1115-1119` → `apps.rs:294-300`）。
- **生产后果**：图片 worker 正在写 4K 大图事务时用户恰好呼出/启动应用 → 主线程写操作 busy 等待，最坏 5s，UI 冻结；典型 SSD 上是几十~几百 ms 的可感卡顿。反向：`save_settings` 十几次独立 fsync（synchronous=FULL 默认）拖长自身窗口，放大撞车概率。数据无损，属可用性问题。
- **建议**：① `save_settings` 的所有 `set_setting` 收进一个事务（一处 commit，锁窗口缩短一个数量级）；② `open_item` 的 `record_app_launch` 挪到已存在的粘贴后台线程或 spawn_blocking（frecency 计数无即时性要求）；③ 显式设 `busy_timeout(1500)` 并在引擎层对主线程写失败给出可重试错误（5s 对 UI 线程过长）。权衡：①③ 改动小收益确定；单写者连接（所有写走一条专用连接排队）是更彻底的方案但改造面大，当前规模不必。
- **置信度**：高（拓扑与默认值核验）；冻结时长的实际分布未实测（见诚实清单）。

### P2-1 clip_max 后端不钳制：直接 invoke 可写入 0/负数/超大值

- **证据**：`save_settings` 对 `stage_max` 有 `clamp(10, 500)`（`lib.rs:514`），对 `clip_max` 直接 `RETENTION_LIMIT.store` + `set_setting`（`lib.rs:497-505`），无钳制。前端 Select 是闭集（`SettingsView.tsx:750-755`，100/500/1000/5000），导入合并也限闭集（`configfile.ts:66、109`）——正常路径安全，但后端对前端传值零防御（同文件里插件开关都做了「未知 id 直接报错」，`lib.rs:565-568`，口径不一）。
- **生产后果**：被篡改的会话或未来前端 bug 传 `clip_max=0` → 每次复制后的保留清理（`clipboard.rs:286-291`）删光全部未固定历史；`clip_max=5000` × 图片 16MB = 理论 80GB 高水位（RETENTION_LIMIT 同时管图片表，`clipimage.rs:134-139`）。
- **建议**：与 stage_max 同口径 `clamp(10, 5000)`，一行修复。权衡：无。
- **置信度**：高。

### P2-2 监听线程 / 图片 worker 死亡静默：eprintln 在 GUI 不可见，无 diag::log、无重启

- **证据**：两个后台线程的每个失败分支（开库失败、arboard 初始化失败、`master.run()` 退出）都是 `eprintln!`（`clipboard.rs:246-249、256-259、316-328`）。项目已有落盘诊断设施 `diag.rs`（`lib.rs:1783` 已 init）但这里没用；vault 的焚除重试倒是全程 `diag::log`（`vault.rs:103-134`）。
- **生产后果**：监听线程一旦死亡（arboard 初始化失败是已知现实场景），剪贴板记录静默停止直到用户重启——用户只会觉得「历史不更新了」，远程诊断时 zap.log 里一片空白。
- **建议**：失败分支改 `diag::log`；外面包一层死亡重启（线程函数循环或由主线程检测 JoinHandle）。权衡：重启循环要防错误风暴（加退避）。
- **置信度**：高。

### P2-3 stage 批量放入未事务化，与同文件自我论证过的成本模型不一致

- **证据**：`commit_add` 逐条 `insert_one`（含每条一次 evict DELETE），全部 autocommit（`stage.rs:94-135`）；而 `remove_batch`/`restore_batch` 的注释明确论证过「500 次锁内 WAL fsync 可达秒级」并因此事务化（`stage.rs:393-411`）。`commit_add` 在引擎 clip 锁内执行（`mod.rs:354-358`）。
- **生产后果**：一次拖入 200 个文件 = ~400 次锁内 autocommit fsync，期间 vault/设置/搜索全部排队；数据本身无损（逐条容错语义也合理）。
- **建议**：整批包一个事务（事务内单条失败回滚当前语句、逐条容错语义不变——remove_batch 已示范该写法）。权衡：无。
- **置信度**：高。

### P2-4 无空间回收：无 VACUUM、无退出 checkpoint，DB 高水位只增不减

- **证据**：全库无 `VACUUM`/`auto_vacuum`/`wal_checkpoint` 调用（全库 grep 仅 `clipboard.rs:205` 一处 PRAGMA）。删除只来自保留清理（16MB BLOB 行会整行删）。本机实测：zap.db 34,000,492 B + zap.db-wal 5,941,072 B（2026-10-29）。
- **生产后果**：SQLite 会复用空闲页，文件大小 ≈ 历史最高水位的平台期——一次「批量截图→淘汰」后 DB 常驻在峰值体积；WAL 正常被 autocheckpoint（默认 1000 页）压制，但进程被强杀时遗留的 WAL 会在下次打开时整体回放，极端下 WAL 可临时膨胀。配合 P0-1，单条巨型内容的高水位是永久性的。
- **建议**：退出时 `PRAGMA wal_checkpoint(TRUNCATE)`（一行）；可选：启动时查 `freelist_count` 超阈值（如 >50% 页数）则后台跑一次 `VACUUM`（注意 VACUUM 需要等量临时空间且重建全部三连接下的库——执行期间要挡住写入，实现成本中等，可推迟）。权衡：auto_vacuum 无法对现有库追溯开启（需重建），不推荐。
- **置信度**：高。

### P2-5 vault 删除是硬删除：无软删/无撤销/无本地快照

- **证据**：`action("delete")` 直接 `DELETE FROM accounts WHERE id=?1`（`vault.rs:392-396`），无回收站、无撤销窗口；恢复途径只有用户先前导出的 .zapvault。
- **生产后果**：误删账号且从未导出 = 永久丢失。个人密码库场景下误删概率不低（列表视图点错行）。
- **建议**：前端加二次确认（交互层）；数据层可选做「删除后 30 天内可撤销」——软删标记列 + 列表过滤，或最轻量：删除前把该行密文写进 `diag` 同级的本地撤销文件。权衡：软删列要过迁移（P1-2 的机制正好用上）；按「便捷优先」拍板，二次确认是底线。
- **置信度**：高（后端行为核验；前端是否有确认框未逐一核验 UI——见诚实清单）。

### P2-6 双删口单语句竞态：保留清理与固定操作的读-改-写不在事务内

- **证据**：`pin_toggle` 是「UPDATE→再 SELECT 读回状态」两语句（`clipboard.rs:474-492`、`clipimage.rs:282-300`），与监听线程的保留清理 DELETE（另一连接）并发时，理论上存在「pin 与被清理同一行」的窗口：清理子查询按 `pinned=0` 过滤（`clipboard.rs:287`），pin 的 UPDATE 先提交则豁免成立，竞态窗口只是 UPDATE 与 DELETE 同时提交时的行锁串行化结果——SQLite 写串行保证两个写事务不重叠，故实际无丢失更新；真正的瑕疵是 `pin_toggle` 返回值的 SELECT 与 UPDATE 之间该行可能已被另一连接删除 → SELECT 返回 Err 被当作操作失败上报，而 UPDATE 实际已生效。
- **生产后果**：极小概率下前端收到错误但条目实际已 pin（状态回显不准）；无数据损坏。
- **建议**：UPDATE 的 `changed>0` 分支里用 `RETURNING pinned`（SQLite 3.35+，bundled 版本支持）一步取回，消除窗口。权衡：无。
- **置信度**：中高（竞态推理基于 SQLite 写事务串行语义，未实测复现）。

### P2-7 配置导入导出边界清晰，但 autostart 语义跨机不成立

- **证据**：配置文件只含设置/插件开关/捷径，明确不含 DB 数据（`appconfig.rs:117-119`），版本化拒新（`appconfig.rs:83、135-148`，前端合并限闭集 `configfile.ts:80-125`）——这套设计是好的。但 `autostart` 被导出为设置项（`lib.rs:611`），导入后走 `save_settings`……实际 `save_settings` 根本不消费 autostart（入参清单无此项，`lib.rs:348-371`），autostart 走独立的注册表命令 `autostart_set`（`lib.rs:583-591`）。即导出文件里的 autostart 字段**导入后没有任何生效路径**。
- **生产后果**：换机导入配置后 autostart 静默不生效——用户以为导入了，开机不自启。属「数据搬家」断点而非数据丢失。
- **建议**：要么导出时去掉 autostart 字段（诚实），要么导入合并后前端检测该键差异并提示用户手动开（或导入流程自动调 `autostart_set`）。权衡：自动调注册表更顺滑，但要处理权限差异。
- **置信度**：高（`save_settings` 入参与 ConfigSettings 逐一比对核验）。

### P2-8（并入诚实观察）数据目录三件套实况

- **证据**：本机 `%APPDATA%\com.ais.zap`：zap.db 34MB、zap.db-wal 5.9MB、zap.db-shm 32KB、zap.log 40KB（轮转 1MB 上限，`diag.rs:14-26`）、capture.log 25KB、icons/ 149 文件。DB 路径 = `app_data_dir`（`lib.rs:1781-1784`）+ identifier `com.ais.zap`（`tauri.conf.json:5`）。
- **含义**：当前规模健康；此条为 P0-1/P2-4 的实测锚点，不单独立项。

---

## 3. 数据类功能或流程更改建议

1. **引入 user_version 迁移器（对齐 P1-2）**：`open_db` 单点 `BEGIN IMMEDIATE` + 事务内重读版本 + 逐步执行 + 写回；把现有两处吞错 ALTER 收编为迁移步骤 1/2。这是后续一切 schema 演进的前置设施，建议最先做。
2. **文本 clip 收口（对齐 P0-1）**：入库长度硬上限 + 首行/换行数入库时物化为列（迁移步骤 3 顺带回填），搜索 SQL 改读列。一并解决膨胀、击键扫描、呼出拖全文三个症状。
3. **补 clip 删除/清空入口（对齐 P1-1）**：`clip_delete`/`clip_clear_unpinned` 两个命令 + 右键入口 + 操作时 `begin_self_copy()` 抑制。这是隐私事故的唯一可交付补救手段。
4. **崩溃/损坏软着陆（对齐 P0-2）**：启动 `quick_check` → 失败改名 `zap.db.broken-*` 换新库 + diag 记录；退出 `wal_checkpoint(TRUNCATE)`；可选 vault 密文自动快照（保留 3 份滚动）。
5. **写路径统一纪律**：`save_settings` 收单事务、`record_app_launch` 出主线程、`commit_add` 事务化、watcher 死亡走 diag::log + 重启（对齐 P1-3/P2-2/P2-3）。同一 PR 内改完，测试面小。

---

## 4. 诚实清单

- **busy_timeout=5000** 核验自本机 cargo registry 的 rusqlite-0.40.2 源码（`E:\rust\cargo\registry\src\rsproxy.cn-...\rusqlite-0.40.2\src\inner_connection.rs` 的 `sqlite3_busy_timeout(db, 5000)`），未抄录精确行号；另两处 `busy_timeout(0)` 属其他配置路径。
- **主线程冻结时长未实测**：P1-3 的阻塞量级是事务成本推算（16MB BLOB WAL 提交），未在真机压测；结论方向可靠，数值置信中等。
- **zap.db 未打开检查**：本机无 sqlite3 CLI，34MB 只能按文件大小引用，表级体积分布（文本 vs 图片 BLOB 占比）未测。
- **NSIS 卸载残留**：`tauri.conf.json` 未配置 `deleteAppDataOnUninstall`（Tauri 2 默认 false，卸载不清 appdata）——默认值来自 Tauri 2 文档知识，未对照 Cargo.lock 里的 tauri 版本逐版核验；仓库内亦无自定义安装钩子。
- **前端是否有 vault 删除确认框**：只核验了后端硬删除（`vault.rs:392-396`），SettingsView 之外的 vault 视图 UI 未逐一读；即使有确认框，P2-5 的「无撤销/无本地快照」仍然成立。
- **capture.log（25KB）来源未追溯**（疑似 hotkey_capture 模块所写），未纳入评审。
- **在途改动**：本轮 git 工作区的未提交文件（JsonWorkbench.tsx / panel.tsx→tsx / ToolWorkbench.tsx）均不在数据层路径上；简报所述 clip 行号寻址改动已提交（ad99093），已按磁盘现状纳入评审（`target=rowid`、`clip_row`/`clip_texts` 回查路径均已核验）。
- **保留清理的 pinned 挤占边界**：若固定条目数 ≥ RETENTION_LIMIT，清理子查询的 top-N 可能全被固定条目占据、导致非固定条目被清空——纯边角，未单独立项，仅供留意。
