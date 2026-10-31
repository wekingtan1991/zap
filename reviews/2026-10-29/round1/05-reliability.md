# 可靠性与测试维度评审 · 第一轮（2026-10-29）

评审人：可靠性与质量工程师。事实源 = 磁盘当前状态（含 5 文件在途改动，已按最新事实审）。只读源码，未修改任何 src 文件。

---

## 1. 角色结论摘要

1. **`panic = "abort"` 改变了整个 panic 审计的结论**：Cargo.toml `[profile.release] panic = "abort"` + `main.rs:2` 的 `windows_subsystem = "windows"`（stderr 全弃）+ 全仓**零 panic hook**（grep `set_hook` 无命中），意味着**任何一个后台线程的任何一处 panic 都会立即静默杀掉常驻进程**——托盘消失、热键失效、剪贴板历史停止，无日志、无自动恢复。其他角色报告的「Mutex poisoning panic」在此配置下反而是不可达的（abort 下锁永远来不及被污染）——真正的杀手是首个 panic 本身。
2. **启动期最现实的 panic 触发器是 `mod.rs:179`**（`open_db().expect`）：DB 损坏/杀软锁定/Profile 只读 → 应用双击无反应，且 zap.log 里什么都没有（diag::init 在 panic 前执行但没人写 panic 信息）。backup.rs 只是 vault 口令加密导出，**不是 DB 恢复机制**；全库无任何损坏恢复路径。
3. **长驻可靠性最大的静默退化源是剪贴板 watcher**：三个失败点（watcher 连接、arboard 初始化、master.run 退出）全部只走 `eprintln!`（release 下丢弃），线程死了就永久死，剪贴板历史无声停摆。叠加**无 busy_timeout**（全仓只有 `journal_mode=WAL` 一条 PRAGMA），双写连接并发时条目静默丢失。
4. **可诊断性不足以支撑远程排障**：zap.log 无级别、时间戳只有 Unix 毫秒，而热键注册失败、watcher 死亡、Everything 拉起失败等关键事件走 stderr（release 全丢）——用户报「热键呼不出来了」时日志里不会有任何线索。
5. **测试金字塔地基扎实但塔尖缺失**：Rust 120 个 `#[test]` 全部是纯函数/内存 DB 单元测试（契约覆盖优秀），vitest 31 个同理；但 drop→入库→开面板、热键失败回滚、watcher 死亡、DB 损坏恢复等**跨 Win32 边界行为零自动化覆盖**，全靠手工。CI（ci.yml）只跑单测+fmt+clippy+构建。

---

## 2. 发现清单

### P0-1 任何线程 panic = 进程静默死亡：abort × 无 panic hook × stderr 不可见，且启动期 DB expect 有现实触发路径

**证据**
- `src-tauri/Cargo.toml` `[profile.release]`：`panic = "abort"`（附带 `strip = true`，崩溃转储也缺符号）。
- `src-tauri/src/main.rs:2`：`#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` —— release 下 stderr 无人接收。
- 全仓 0 处 `std::panic::set_hook`（grep 证实），diag.rs（providers/diag.rs:10-39）只记录 vault 焚除与 stagedrop 事件，不含进程生命周期。
- 生产代码可达 panic 点（非测试，逐个判定）：
  - `providers/mod.rs:179` `open_db(db_path).expect("打开剪贴板数据库失败")` —— **现实触发**：zap.db 损坏（磁盘坏块/强断电 checkpoint 中断/杀软写入干扰）、目录只读、AV 短暂独占。`Connection::open` 是惰性的，失败在 `execute_batch(PRAGMA journal_mode=WAL;...)`（clipboard.rs:204-211）才爆。在 `lib.rs:1785` setup 内 panic → abort → **应用启动即消失，zap.log 无记录，无对话框**。
  - `providers/files.rs:21` `Client::builder().build().expect`（OnceLock::get_or_init 内）——TLS 后端初始化失败即 abort，低概率。
  - `providers/stage.rs:228` `h.join().expect("stage finish worker panicked")` —— finish_one panic 会升级为整进程 abort。
  - `providers/clipboard.rs:433` 与 `providers/vault.rs:240` `partial_cmp(&a.0).unwrap()` —— 当前打分源头（50.0-gap、60.0-i）产生不了 NaN，属潜在雷：未来任何打分改动引入 NaN = 每次搜索 abort。
  - `lib.rs:1966` `default_window_icon().unwrap()` —— icon 由 tauri.conf.json 编译期固定（icons/ 齐全），现实不可达；`lib.rs:2022` `.run().expect` 为 Tauri 惯用终点（WebView2 缺失时 NSIS 默认 bootstrapper 先行安装）。
- **Mutex poisoning 家族（~150 处 `expect("... poisoned")`）在 abort 语义下不可达**：污染需要某个线程在持锁时 unwind，而 abort 下首个 panic 直接杀进程。它们不是独立风险，而是「真风险 = 首个 panic」的佐证。仓库其实已有正确姿势：`stage.rs:278` `lock().unwrap_or_else(|e| e.into_inner())`（污染容忍），仅此一处。

**生产后果**：常驻进程崩溃后用户视角 = 托盘图标消失、全局热键失效、剪贴板历史停止——「用户无感知坏掉」的原型。autostart 只在登录时拉起，会话中崩溃后**不会自动恢复**；崩溃原因既不落 zap.log 也无 WER 转储配置（strip 且无 LocalDumps 注册）。DB 损坏场景下更糟：应用拒绝启动 + 无诊断 + 用户唯一出路（删 zap.db）会连带丢掉**同一个库里的账号库**（vault/clip/appconfig/stage 同库，mod.rs:180-183）。

**建议与权衡**：
1. `std::panic::set_hook`：把 panic 信息（message + location + 线程名）写入 zap.log 再走默认 abort——hook 在 abort 前仍会执行，这是零行为风险的纯增益。
2. `SearchEngine::new` 的 open_db 失败降级：先 `PRAGMA integrity_check`；失败则把 zap.db 改名 `zap.db.broken-<时间戳>` 后重建空库启动，zap.log 记录迁移事件并提示用户（vault 可从 `.zapvault` 备份恢复）。权衡：牺牲已损历史换可用性与可诊断性；vault 数据在 broken 文件里保留（专业用户可用 sqlite3 .recover 抢救），优于现在的「整进程拒启 + 手删库」。
3. 备选/叠加：改回 `panic = "unwind"` 让后台线程 panic 不杀进程——但这改变二进制体积与语义面，且 unwind 后 poisoned 状态遍布 150 处 expect，反而扩大爆炸半径；**不推荐先做这个**，先做 hook+降级。

**置信度**：高（abort/hook 缺失/expect 位置均为代码事实；DB 损坏的现实频率为中，但一旦发生即 P0 后果）。

### P1-1 剪贴板 watcher 三处静默死亡，无重启、无日志、无用户信号

**证据**：`providers/clipboard.rs:240-331`
- `:247` 图片 worker `open_db` 失败 → `eprintln!` → return（线程死，img_tx 永久满/断）；
- `:257` watcher 主连接 `open_db` 失败 → `eprintln!` → return（历史功能整体死亡）；
- `:314-319` `arboard::Clipboard::new()` 失败 → `eprintln!` → return；
- `:322-329` `Master::new` 失败或 `master.run()` 退出 → `eprintln!` —— clipboard-master 的消息循环一旦退出（其内部窗口被销毁/错误传播）**不会自愈**；
- `:305-308` `on_clipboard_error` 同样只 eprintln。

**生产后果**：长驻数天的进程 + 剪贴板是全局独占资源 + 与一切常驻软件（输入法、安全软件、其他剪贴板工具）共处——监听死亡是长尾必现事件。死后的表现是「剪贴板历史慢慢变旧」，用户在数小时后才发现，远程排障时 zap.log 零记录。

**建议与权衡**：把三处 eprintln 换成 `diag::log`（含错误串）；`master.run()` 返回后 respawn 整个 watcher（带 1 次退避，防死循环）；spawn 成功/失败各记一条启动日志（当前 watcher 活着还是死了，日志上完全不可见）。权衡：respawn 要处理 db 连接重建与队列重建，改动集中在 spawn_watcher 内部即可。

**置信度**：高（代码事实）；master.run() 实际退出的触发频率未知（见诚实清单）。

### P1-2 全仓无 busy_timeout：双写连接并发冲突 = 静默丢剪贴板条目 / 设置保存偶发失败

**证据**
- 全仓唯一 PRAGMA 是 `providers/clipboard.rs:204-205` 的 `journal_mode=WAL`；grep `busy_timeout|busy_handler` 零命中（rusqlite 默认 0ms）。
- 写连接有两个：watcher 专属连接（clipboard.rs:254）与 engine 的 `Mutex<Connection>`（mod.rs:165）。WAL 允许单写者：两边同时写 → 后到者立即 `SQLITE_BUSY`。
- watcher 写路径吞错：`clipboard.rs:280-291` `let _ = self.conn.execute(INSERT...)`——BUSY 时**该条剪贴板静默不入库**；图片侧 `clipimage::store` 返回 `.ok()?`（clipboard.rs:230）同样静默。
- 主线程同步命令也写库：`open_item` → `record_app_launch`（lib.rs:1115-1118）；`save_settings`（lib.rs:348，注释自认主线程 SQLite 写毫秒级）——与 watcher 并发写即冲突，表现为设置保存报错（用户可见一次「保存失败」）或启动计数丢失。

**生产后果**：快速连复制的场景下丢历史条目（无感知坏掉）；vault_add/vault_update 偶发 Err（用户重试即过，但观感是「随机失败」）。

**建议与权衡**：在 `open_db` 的 execute_batch 里加 `PRAGMA busy_timeout=2000;`（一行改动，两个连接共用 open_db）。2s 上限对主线程路径偏长，可拆「监听线程连接 2s / 引擎连接 500ms」两档；最小动作先加统一 500ms。权衡：几乎无成本，唯一顾虑是主线程写等锁 500ms 的手感——WAL 下写冲突窗口本就毫秒级，500ms 只是上界不是常态。

**置信度**：高（机制确定）；冲突的现实频率 = 中低（写事务都极短），但后果是静默数据丢失，按生产标准必须收口。

### P1-3 关键失败事件走 stderr：release 下热键失效、watcher 死亡、Everything 拉起失败全部无日志

**证据**（eprintln! 在 release GUI 下全部丢弃）：
- `lib.rs:1854` 呼出热键注册失败（仅托盘可呼出的降级，用户最需要日志的场景）；`lib.rs:1884` 直达热键注册失败；`lib.rs:1895` 透明特效失败；`lib.rs:1674` 热键事件不匹配。
- `clipboard.rs:247/257/306/317/325/328`（见 P1-1）。
- `everything.rs:330-342` 自动启动流程全程 eprintln；`files.rs:94/101/106` files 源请求失败。
- 对照组：vault 焚除路径已正确落 diag::log（vault.rs:100-134），stagedrop 生命周期已落（stagedrop.rs:206/392/401/426/596/683/825/860/882）——说明落日志的纪律存在，只是没覆盖全。

**生产后果**：zap.log 现状只够诊断 stagedrop 与焚除。用户报「热键呼不出来 / 文件搜不到 / 不记录剪贴板了」三类最高频故障时，日志均无证据。另外 `hotkey_capture.rs:18-35` 自建了第二个日志（capture.log，格式是 `SystemTime` Debug、无轮转、与 zap.log 不同目录规范）——双日志体系增加远程排障负担。

**建议与权衡**：把全部 eprintln 收敛进 diag::log，日志行加级别前缀（`[E]`/`[W]`/`[I]`）+ 统一模块前缀（现状已有 `stagedrop:` 等前缀，推广即可）；启动时记一条 `startup version=0.13.0 hotkey=... clip=...`（版本号是远程排障第一问）；capture.log 并入 zap.log。时间戳可保留 Unix 毫秒（脚本可读），另加人类可读列成本极低。权衡：search 热路径禁刷日志的既有纪律保留（files.rs:81-83 的做法是对的）。

**置信度**：高。

### P1-4 超级面板抓取的失败分支跳过剪贴板现场恢复，用户原内容被覆盖

**证据**：`providers/superpanel.rs:61-89`：先存 `orig_text`/`orig_img`（:61-71）→ 注入 Ctrl+C（:72）→ 轮询序列号（:75-83）→ `let Some(text) = ...get_text()... else { return }`（:84-89）。序列号已变（复制已成功写剪贴板）但 `get_text` 失败时（典型：Explorer 里选中**文件**复制，剪贴板是 CF_HDROP 不是文本），提前 return，**跳过 :91-107 的恢复逻辑**——用户原剪贴板内容已被「选中的文件引用」覆盖且不恢复。恢复逻辑只在成功读到文本后才执行。

**生产后果**：用户剪贴板内容丢失（非磁盘数据，但属于「用户无感知的数据破坏」）：用户复制了密码/文本 → 顺手触发超级面板时鼠标恰在文件选区上 → 原复制被静默替换。密码场景下用户常无察觉。

**建议与权衡**：把「恢复现场」移到所有 return 路径之前统一执行（或用 defer/守卫结构）：超时路径其实无需恢复（序列号未变，剪贴板未被改），但「seq 变了且 get_text 失败」路径必须恢复（文本回填 orig_text 或图回填 orig_img）。改动 ~10 行，可补一个纯函数单测（seq 变化 × get_text 成败 × 恢复调用的真值表）。

**置信度**：高（代码路径明确）。

### P1-5 DB 损坏 / 磁盘满 / 只读：恢复路径缺失，vault 安全完全依赖用户自觉导出

**证据**
- 单库承载 clipboard_history、clipboard_images、accounts(vault)、app_settings、stage_items、plugin_states（mod.rs:178-183 六个 init 全打在同一连接）。
- `providers/backup.rs` 全文 = vault 条目的 PBKDF2+AES-GCM 加密导出/导入，**不是 DB 备份**；无任何自动备份、无 integrity_check、无损坏恢复流程。
- 磁盘满/只读：diag::log 静默忽略失败（diag.rs:27-33，设计如此）；watcher 写入 `let _ =` 静默（clipboard.rs:280）；设置写入返回 Err（用户可见）。DB 不会因此损坏（SQLite FULL 错误安全），但表现 = 静默丢条目 + 偶发报错，无可诊断线索。
- vault 只有 `.zapvault` 手动导出一条防线（lib.rs:965-974）；无导入提醒、无「上次备份时间」。

**生产后果**：与 P0-1 的启动路径叠加：DB 损坏 → 拒启 → 用户手删库 → **账号库全灭**（除非曾导出备份）。即使做了 P0-1 的改名降级，vault 仍在 broken 文件里需要恢复流程。

**建议与权衡**：① P0-1 的损坏降级里同时提示「账号库可能可从备份恢复」，前端加一句恢复指引；② 低价高收益：vault 每次变更后把 `.zapvault` 自动写一份到 app_data_dir（无口令加密不可行——那就退而求其次只在导出路径上做「上一次导出时间」显示 + 30 天未导出提醒）；③ `open_db` 成功后跑一次 `PRAGMA quick_check`（毫秒级，WAL 读）结果落日志，给远程排障一个基线。权衡：自动备份涉及口令管理，违背「便捷优先」拍板，故只建议提醒不强制。

**置信度**：高（结构事实）；损坏发生率中低，但 vault 后果权重极高。

### P2-1 Everything 端口探测可被任意本地 HTTP 服务劫持，且 JSON 失败路径不清缓存不退避

**证据**：`providers/files.rs:37-50`：`resolve_port` 对候选端口 [托管 ini, 8080, 80] 发 `?json=1&n=1&s=zap-probe`，判定 = `send().is_ok()`——**任何** HTTP 响应（404 也算）即缓存为 PORT。`search()`（:93-107）里只有 `send` 失败才 `PORT.take()`；JSON 解析失败/无 results 字段路径既不清 PORT 也不 `mark_failed()`。

**生产后果**：本机 8080/80 上有 dev server / XAMPP / 路由管理映射时，文件源被劫持：搜索每击键都白打 250ms 超时请求（spawn_blocking 内不卡 UI 但浪费且永不自愈），全盘搜索永远空。降级路径存在（其他源正常），故 P2。

**建议与权衡**：probe 判定加一层「响应是合法 JSON 且含 results 键」（一次 `resp.json::<Value>()` 成功即可）；search 的 JSON 失败路径同样 `PORT.take()` + `mark_failed()`。改动 ~10 行。

**置信度**：高。

### P2-2 测试金字塔：单元层优秀，Win32 边界与链路层零自动化，崩溃/恢复路径完全靠手工

**证据**
- Rust 120 个 `#[test]`（grep 计数；brief 记 117+1，在途改动后 120）：覆盖 open_db/迁移（:memory:）、DIBV5 字节布局（clipboard.rs:584-608）、图片队列 worker 内联驱动（clipboard.rs:656-671）、行号寻址线协议契约（clipboard.rs:677-696）、stagedrop decide 真值表（stagedrop.rs:895+）、corner_origin/avoid_drag_origin 几何、backup 加解密往返（backup.rs:130-197）、TOTP RFC 向量（totp.rs:99-140）、appconfig 前后向兼容（appconfig.rs:164+）——纯函数与内存 DB 层纪律极好。
- vitest 31 个（7 个文件）：appearance/configfile/password/commands/dispatch/registry/types，同为纯逻辑。
- **零覆盖**（绑定具体行为）：① drop→begin_processing→stage_add→finish_processing→open_stage_panel 全链路（stagedrop 只有 decide 纯函数与文案分桶）；② watcher 死亡/自愈（P1-1）；③ DB 损坏恢复（P0-1/P1-5，open_db 只测过 :memory:）；④ 热键注册失败回滚（lib.rs:374-495 六个分支，依赖 RegisterHotKey）；⑤ 图标提取串行锁与失败不缓存纪律（stage.rs:266-278，真机踩过的坑无回归测试）；⑥ CI（.github/workflows/ci.yml）无任何起 GUI 的冒烟步骤。
- 亮点值得点名：行号寻址改造（在途改动）补了 Rust 契约测试 + 前端 types.test 反向锁定，这是好范式。

**建议与权衡**：不必上 WinAppDriver/E2E 框架（维护成本对本项目不现实）。性价比排序：① `open_db` 加真实临时文件 + 逐字节破坏的损坏用例（纯 Rust，立刻可写，锁 P0-1 降级行为）；② `files::resolve_port` 劫持用例（mock 本地 HTTP）；③ `save_settings` 热键回滚分支用 trait 抽象 RegisterHotKey 后可测（工程量中）；④ 发布前手工冒烟清单落档（drop→面板、热键四槽、睡眠唤醒）——把口头纪律变 checklist。

**置信度**：高。

---

## 3. 可靠性类功能或流程更改建议

1. **panic hook + 启动自检 + DB 损坏降级**（对应 P0-1/P1-5）：set_hook 落 zap.log；open_db 失败 → integrity_check → 失败改名 broken 重试 → 日志 + 前端提示。这是唯一一条 P0 的收口，且全部是启动路径内的局部改动。
2. **`PRAGMA busy_timeout` 进 open_db**（对应 P1-2）：单行 PRAGMA，消除静默丢条目类。
3. **日志收口**（对应 P1-3/P1-1）：eprintln 全量迁 diag::log；加级别 + 启动行（版本/热键/各子系统状态）；watcher 三处死亡改日志 + respawn。目标：用户发来一个 zap.log 能回答「热键注册了吗、watcher 活着吗、everything 到什么状态、DB 检查结果」四问。
4. **超级面板恢复现场守卫**（对应 P1-4）：恢复逻辑改为所有 return 路径统一执行，配真值表单测。
5. **发布冒烟 checklist 落档 + 两个纯 Rust 恢复用例**（对应 P2-2）：把已踩过的坑（SHGetFileInfoW 并发、焦点/IME、DIBV5）对应的回归测试补到「任何环境重跑都过」的程度，手工项（drop 链路、睡眠唤醒）落成 checklist 随 release 走。

---

## 4. 诚实清单

- **睡眠唤醒**：机制上 RegisterHotKey（global-shortcut）与 clipboard-master 的监听窗口都跨睡眠存活，但未真机验证唤醒后两者回调是否 100% 存活；代码无 WM_POWERBROADCAST 处理（grep 证实），若插件内部有掉注册场景则无自愈。建议列入手工冒烟。
- **多显示器拔插**：布局每次会话起点实时查 MonitorFromPoint（stagedrop.rs:556-568）与 toggle_panel 实时查（lib.rs:1478-1489），天然自适应；但拖拽进行中拔显示器，条子可能滞留离屏坐标直到下次布局——失效方向有界（下轮会话自愈），未实测。
- **DPI 缩放运行时变更**：stagedrop.rs:569-577 每次 layout 现取 scale，lib.rs:1484 同理，理论上自适应；未实测 100%→150% 热切换后面板尺寸。
- **stage::finish 并发线程峰值**（stage.rs:221-230）：单次最多 500 个 scoped 线程（注释已说明成本模型），栈是惰性提交不构成常驻泄漏，但内存瞬时峰值未实测；500 条全为网络死链时的最坏时延=最慢单条（设计已知）。
- **clipboard-master `master.run()` 的现实退出频率**：未知，无公开的已知退出条件清单；respawn 建议基于「退出是可能的」防御立场而非观测到的故障。
- **files.rs TLS 客户端初始化失败**（files.rs:21）的现实概率：reqwest 0.13 + default-tls(schannel) 下极低，未构造出复现。
- **lib.rs:1966 `default_window_icon().unwrap()`**：icon 由 tauri.conf.json 编译期固定且仓库齐全，判定为现实不可达；若未来改配置需同步。
- **Windows 版本支持矩阵**（代码事实 + 推断）：用到的 Win32 API——CF_DIBV5/BITMAPV5HEADER（Win2000+）、IDropTarget/RegisterDragDrop/RevokeDragDrop/DragQueryFileW（95+）、SHGetFileInfoW/DestroyIcon/GetDIBits（XP+）、DPAPI CryptProtectData（Win2000+）、DwmGetWindowAttribute(DWMWA_CLOAKED)（Vista+）、GetClipboardSequenceNumber（XP+）、GetAsyncKeyState/SendInput（2000+）、SysDragImage FindWindow（无文档契约，XP 时代至今存在）。唯一版本分叉是 Acrylic 特效（tauri WindowEffect，Win10 1607+ 才有效），已有 Err 回退 + 偏好落 0（lib.rs:1894-1901）。**瓶颈不在代码在 WebView2**：微软已终止 Win7/8.1 的 WebView2 支持，现实最低支持 = Win10 1803+；代码无显式版本检查，README 也未声明最低版本——建议文档化（本项目未声明即按「Win10 1803+」口径）。
- **在途改动（行号寻址）**：已按磁盘状态审；其契约测试（clipboard.rs:677-696、types.test）质量好，未发现该改动引入的新 panic/泄漏路径。
- 评审期间在仓库根目录生成的临时文件 panics.txt 已删除，未动任何源文件。
