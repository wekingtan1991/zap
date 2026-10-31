# 13 · 落地可行性工程师 · 幸存项合并与分阶段路线图（2026-10-29）

第三阶段合并报告。准绳：技术类发现以红队裁决（11-redteam）为准绳，产品类提案以怀疑者裁决（12-skeptic）为准绳；红队未裁的 R6 两条 P0 按怀疑者裁决计级。事实源 = 前两轮报告 + 简报，本阶段不做代码复验。

**定级口径**（沿用前两轮实证共识）：P0 = 进程死 / 数据丢 / 无法自救 / 分发第一触点断裂（产品）；P1 = 必然被撞上的可恢复事故与结构性负债；P2 = 卫生、顺手、防御性。量级：S=一天内、M=两三天、L=一周级。

**幸存项统计：P0 × 3 · P1 × 20 · P2 × 18（合计 41 条）。** 其中 2 条含待仲裁成分（P1-02 级别、P2-09 修法），见 §4。

---

## 1. 幸存发现总表

### P0（3 条）

| 编号 | 标题 | 来源（裁决） | 一句话问题 | 修复方案摘要（被裁决认可版） | 量级 | 依赖 |
|---|---|---|---|---|---|---|
| **P0-1** | 进程静默死亡与库损坏无软着陆 | R5 P0-1（维持）+ R8 P0-2（维持）+ R5 P1-5（改写并入）+ R9 P1-3（panic hook 部分并入）+ R4 P1-3（hook 部分并入；expect 风格统一降 P2 顺手）【矛盾⑧合并：两个成分都是真 P0】 | `panic="abort"` × `windows_subsystem` × 零 hook 使任意 panic 静默杀进程且 zap.log 无痕；库损坏时 `open_db().expect`（mod.rs:179）拒启，用户唯一出路手删库连 vault 一起丢 | ① `run()` 挂 `std::panic::set_hook` → `diag::log`（abort 前 hook 仍执行，零行为风险）；② open_db 失败 / `quick_check` 失败 → 改名 `zap.db.broken-<ts>` 重建空库启动 + diag 记录 + 前端提示「账号库可从 .zapvault 恢复」；③ 上次导出时间显示 + 30 天未导出提醒；④ 顺手（P2）：主线程 4 处 `expect("...poisoned")` 改 `unwrap_or_else(|e| e.into_inner())` 风格统一 | M | 无；是 P1-10/P1-11/P2-16 的日志底座 |
| **P0-2** | 分发信任断层（「分发可信」专项） | R6 P0-1 + R9 P1-1；怀疑者 #15 改写四步序（红队不裁产品定级） | 未签名 setup + 零替代渠道，SmartScreen/Defender 在第一触点拦截；SendInput/DPAPI/剪贴板监听/自启/下载提权的高启发式画像加重误报 | 四步序：① release.yml 附安装包 SHA256（零成本）；② winget 社区清单 PR（零成本，注意与 P1-15 版本稳定联动）；③ 签名证书渠道调研（Certum/SignPath/Azure Trusted Signing，半天）；④ 签名落地挂 1.0 门槛。**portable zip 降级改期**（WebView2 引导第二支持面）；微软误报提交为并行外部动作 | S（各步均 S，外部流程为主） | P1-15（winget 要求版本可预测） |
| **P0-3** | 首启引导缺失：呼出热键产品内零露出 | R6 P0-2；怀疑者 #11 改写（欢迎条驳回、tooltip 升主方案）+ #12（空态死文案）+ #17（超级面板露出并入） | 呼出热键唯一露出在设置页，而进设置的前提恰是先呼出；热键注册失败 release 下仅 eprintln 零信号 | 怀疑者改写版：① settings 无 `first_run_done` 键即首启 → 自动呼出一次面板；② 托盘 tooltip 动态化（正常「zap — <热键> 呼出」，失败「热键被占用：左键点此呼出，右键进设置修改」）；③ 删除 App.tsx:1277 死文案（footer 已常驻 pw 提示）；④ 超级面板在设置文案层提一句（不做卡片）。**欢迎条方案已驳回** | S | 热键失败 diag 与 P1-10 共用一次改动 |

### P1（20 条）

| 编号 | 标题 | 来源（裁决） | 一句话问题 | 修复方案摘要 | 量级 | 依赖 |
|---|---|---|---|---|---|---|
| **P1-01** | 键盘面纪律 v2：一次收口六处 | R2 F1/F2/F3（均维持 P1）+ F4/F9/F5（P2 顺路）；怀疑者 #31 采纳 | 右键后键盘整面哑掉（同根因第 4 例）、合并键盘面泄漏进设置页劫持备份弹窗 Enter、useEscCapture 缺 IME 卫兵 | ctxMenu 键盘语义提为独立 window 捕获（互斥显式化，主 onKeyDown 的 ctxMenu 分支删除）；merge 键盘面加 `view.kind !== "main"` 守卫或并入 View 状态机（`{kind:"main",merge}`，推荐后者）；`useEscCapture` 首行加 isComposing（六处消费点全受益）；SuperPanel 补 capture+卫兵；shown 复位点补 `setCtxMenu(null)`；panel.tsx:6-7 契约注释写全四口径；onKeyDown grep 自检进 CI 白名单。**F3 级别含待仲裁成分**（§4-5） | M | 无；P1-05 与它同在 App.tsx 键盘区，宜同批 |
| **P1-02** | 工具面板空态「按 Ctrl+V」监听错位 | R7 P0-1（红队降 P1 / 怀疑者 #20 维持 P0 → **分歧点 §4-1**） | 新用户按空态文案按 Ctrl+V 第一次必然哑火：paste 监听仅在 `awaitPaste===true` 挂载，文案里的并列选项实际是唯一通路 | 监听常驻（面板挂载期 window 捕获 paste，panel.tsx 注释自证非可编辑焦点可达）；「粘贴替换」按钮降级为等待态指示；**常驻后 5 秒超时语义需重设计**（怀疑者补的注意点：常驻监听不需要超时） | S | 无 |
| **P1-03** | 设置页双生效模型 + Esc 丢脏态 | R7 P1-2（维持 P1）；怀疑者 #21 采纳方案① | 改了热键/捷径/开关按 Esc 全部静默丢弃，且外观即时生效、其余保存生效的双模型无法建立心智 | 方案①：脏标记 + Esc 确认条（复用 zap-dialog，「放弃更改/留在页面」）+ 保存按钮无更改时置灰；**不取**全即时保存（热键录制/捷径编辑天然产生半成品）。顺路：App.tsx:232-234 refreshAppearance 条件改「从 settings 退出」（F8，一行） | M | 无 |
| **P1-04** | vault 鼠标面缺口：死右键 + 网址无打开路径 | R7 P1-3（维持 P1）；怀疑者 #22 采纳 | vault 是唯一无 ctxActions 的 kind（右键点了毫无反馈）；「打开网址」唯一入口是 Alt+1 无鼠标等价物 | vault kind 加 ctxActions（查看密码/打开网址/编辑/置顶/删除，全复用既有 vault_action 与右键渲染机制）+ 详情页网址行尾加「打开」按钮（复用 open_item kind="url"） | S | 无 |
| **P1-05** | 合并模式作用域收敛 | R7 P1-4（维持 P1）+ P2-5/P2-9（怀疑者 #23 采纳禁用态三件套） | 合并模式下点非 clip 行直接启动/打开文件，破坏「点=选」心智且有真实副作用；footer Esc 提示失真 | 非 clip 行禁用态（降透明度+点击无效+说明「仅文本剪贴板可合并」）；footer 右区 Esc 提示随 mergeMode 三元切换；「合并粘贴」chip 仅在列表含 clip 条目时显示。窄面板提示精简（P2-9 剩余半条）为可选顺路 | S | 与 P1-01 同批（同文件同键盘区） |
| **P1-06** | 超级面板恢复路径三缺陷 | R1 P1-1（维持）+ R5 P1-4（维持）+ 红队新-2（P2 同函数顺路）；怀疑者 #27 采纳 | 恢复走 arboard set_image 秒级阻塞（项目自测 39MB→5-6s）；seq 已变但 get_text 失败时跳过恢复（用户原剪贴板被覆盖）；动作识别对全文裸跑 serde_json 无 256KB 卫兵 | 恢复分支换既有 `set_image_dib`（clipboard.rs:54 现成）；恢复现场统一到所有 return 路径（守卫结构）+ seq×get_text 真值表单测；`superpanel.rs:186` 解析前加 MAX_INPUT_BYTES 卫兵；`trigger_from_hotkey` 加耗时 diag（R1 建议1） | S | 无 |
| **P1-07** | 主线程写路径与 open_item 点查收口 | R1 P1-3（维持，红队修正：WAL fsync 属 app 分支的 record_app_launch 非 clip 读）+ R8 P1-3（维持）+ R5 P1-2（驳回并入：默认 busy_timeout 已是 5000ms）+ R8 P2-3（stage 事务顺路） | open_item 同步命令在主线程做 clip 全文点查与 record_app_launch 写；save_settings 十几次独立 autocommit 写拉长主线程窗口 | clip 文本分支复用图片分支两段式（同步只做一行 PK 探存在保「已被清理」报错语义，全文读挪进 paste_text_async 已有后台线程）；record_app_launch 挪 spawn_blocking（frecency 无即时性要求）；save_settings 全部 set_setting 收单事务；显式 `busy_timeout(1500)`；stage commit_add 整批一事务（remove_batch 已示范写法） | M | 无；与 P1-08 同触 open_db 但可先行 |
| **P1-08** | 剪贴板存储收口（文本上限/图片预算/空间回收/锁时长） | R8 P0-1（降 P1，机制维持）+ R1 P1-2（维持）+ 红队新-3（P1 锁面加重）+ R8 P2-4/P2-1/P2-6 + R1 P2-8/P2-9；怀疑者 #29/#33 采纳【红队矛盾③：同一修复单】 | 文本单条无上限（一次事件抬升数百 MB 且每击键 top-300 全文 replace 扫描、把 clip 锁持有时长转嫁为主线程排队）；图片 500×16MB=8GB 理论上限、1-3GB 稳态渐进；无 VACUUM/checkpoint 永不回落 | 文本入库硬上限（约 1MB，超限拒收并在文档/UI 说明）；nl_count/head 入库物化为列（迁移步骤，搜索 SQL 改读列，顺带把锁持有时长压回毫秒级——新-3 的处置）；图片表按「总字节预算+条数」双阈值淘汰（默认如 256MB/50 条，设置文案说清截图是主因）；`synchronous=NORMAL`；退出 `wal_checkpoint(TRUNCATE)`；clip_max 后端 `clamp(10,5000)`；pin_toggle 改 `RETURNING`；空查询 latest 拆 substr 版 | M | **依赖 P1-14**（物化列走迁移器） |
| **P1-09** | 密码入库防护与用户补救 | R3 P1-2 + R8 P1-1（红队矛盾⑦合并维持 P1）；怀疑者 #32/#34 采纳（#34 强化） | 非名单来源（浏览器内置 PM 等）密码明文入库、驻留至多 500 条滚动窗口、无任何删除/清空补救命令 | 双管齐下一 PR：预防 = watcher 入库前与 vault 密码集比对（解密缓存随 vault_add/update/import/backup_restore 失效），命中跳过并清历史同值行；补救 = 新增 `clip_delete`/`clip_clear_unpinned` 命令 + clip 行右键入口（进抑制窗防自收录）；#34 强化 = 「记录剪贴板历史」关闭时也提供清空存量动作；vault_import 成功后按导入原文等值清理历史（R3 P2-10） | M | 无；不违「便捷优先」拍板（防历史回显非防查看） |
| **P1-10** | 关键失败走 stderr：release 下零日志 | R5 P1-3（维持）+ R6 P1-5（#14 采纳）+ R9 P1-3（非 hook 部分）；怀疑者 #13 配套 | 热键注册失败、Everything 拉起失败、files 请求失败等最高频故障事件全走 eprintln，zap.log 回答不了「热键注册了吗/DB 状态如何」 | eprintln 全量迁 `diag::log`（保留 stderr 双写）；日志加级别前缀 `[E]/[W]/[I]` + 模块前缀；启动时记一条 `startup version=… hotkey=… clip=…`；capture.log 并入 zap.log（hotkey_capture.rs）；search 热路径禁刷日志纪律保留 | M | 依赖 P0-1 的 diag 底座；与 P1-11 同基建 |
| **P1-11** | watcher 静默死亡无自愈 | R5 P1-1（维持）；怀疑者 #36 采纳（工程侧） | 三处失败点（open_db、arboard、master.run 退出）只 eprintln，线程死了永久死，剪贴板历史无声停摆 | 三处 eprintln 换 diag::log；`master.run()` 返回后 respawn 整个 watcher（1 次退避防风暴，改动集中在 spawn_watcher 内部）；spawn 成功/失败各记一条日志 | S | 依赖 P1-10（日志形态先行） |
| **P1-12** | Everything 无认证 HTTP 服务 + seed_ini 复活 | R3 P1-1（维持加重）+ 红队新-1（P2 放大器并入）；怀疑者 #32 采纳 | zap 主动把 Everything 配成本机无认证全盘文件索引 HTTP 服务；用户手动关掉会被 seed_ini 静默改回 | seed_ini 生成随机用户名/口令写 `http_server_username/password`，files.rs 请求带 Basic Auth（端口+凭证内存持有，与现有 ini 读取同路径）；seed_ini 改「只补缺、不覆盖既有值」（尊重用户关闭，新-1 的最小修）；SDK DLL 方案不做（改动大） | S | 无；不影响 zap 自身查询体验 |
| **P1-13** | 受保护复制通道不一致 | R3 P1-3（维持加重）+ R3 P2-8/P2-10（同族打包）；怀疑者 #32 采纳【R3 建议2 的一个 PR】 | 密码/TOTP 走 copy_text_private，但备注/用户名/超级面板复制走普通通道（进 Win+V 与云剪贴板）；焚除窗口内快速粘贴可把密码注入前台 | notes/username/superpanel 三处改 `copy_text_private`（现成函数一行级）；焚除窗口期内（seq 判定）快速粘贴跳过注入或退化为回源历史；vault_import 清理已并入 P1-09 | S | 无 |
| **P1-14** | user_version 迁移机制缺失 | R8 P1-2（维持）；怀疑者 #33 采纳 | 无版本标记，吞错 ALTER 只覆盖加列；破坏性变更无执行点，ALTER 静默失败导致 search 空列表级降级 | open_db 单点引入 `PRAGMA user_version` 迁移器：`BEGIN IMMEDIATE` → 事务内重读版本 → 按 `v < N` 逐步执行 → 写回 → commit（三连接并发天然幂等）；现有吞错 ALTER 收编为步骤 1/2；约 50 行 + 幂等测试 | S | 无；**是 P1-08 的前置**，也是未来一切 schema 演进（含已降级的软删）的设施 |
| **P1-15** | 版本语义空心化（CHANGELOG + 1.0 门槛） | R6 P1-6（#16 采纳）+ R9 P2-2（版本校验脚本并入） | 14 天 13 版、release notes 恒定、无 CHANGELOG；版本四处 bump 纯手工且 release 只校验一处 | CHANGELOG.md（Keep a Changelog，git log 回补 13 版）；release.yml 生成 per-tag notes；节奏改功能攒批+语义化；`scripts/verify-version.mjs` 断言四处版本相等接线 pre-push/CI/release.yml；README 测试数去硬编码 + CONTRIBUTING 改四处；README 补插件生态立场一句话（R6 P2-9 的降级形态顺路） | S | 无；与 P0-2 winget 联动 |
| **P1-16** | 诊断出口：设置页新增「关于」区 | 怀疑者新发现1（结构性缺口）+ R6 P1-3 + R9 P1-3 + R6 P2-10（#13/#19 采纳，改写形态） | SECURITY.md 引用的「设置→关于」不存在；版本/数据位置/诊断三要素无家可归；bug 模板不引导取日志 | 设置页新增「关于」区：版本号 +「打开数据文件夹」按钮（一条 opener 命令）+「反馈请附 %APPDATA%\com.ais.zap\zap.log」说明；SECURITY.md 引用随之落地（修文档不如修结构）；bug_report.yml 加日志路径项、版本占位符改现值；**排除**托盘诊断项与一键导出 zip（怀疑者裁决） | S | P0-1（panic hook）完成后此区即有内容可看 |
| **P1-17** | lib.rs 拆分五步（含解反向边 + 策略提纯） | R4 P0-1（降 P1）+ R4 P1-5（维持）+ R4 §3.3 提纯清单 + R4 P2-9/P2-10（顺路） | lib.rs 2024 行零测试，失焦即隐策略（三次事故家族防线）唯一无锁；三条反向依赖边使 crate 根无法拆 | R4 §3.1 五步路线原样执行：0 解三条反向边（stagedrop.rs:889 下沉、superpanel.rs:279 改中间层、lib.rs:90 改 stagedrop API）→ 1 settings.rs → 2 state.rs → 3 panel.rs + `should_hide_on_blur` 提纯 + 三事故场景单测（顺带 `should_skip_paste_backfill`/`should_close_panel` 提纯与 paste 编排收口）→ 4 commands.rs + spawn_blocking helper。每步独立提交可 revert，命令名/线协议不变 | L | 步 0 无依赖；步 3 依赖步 0/2；与 P1-18 冲突面最小（不同段） |
| **P1-18** | 设置链路九处镜像收口（含锁序显性化） | R4 P0-2（降 P1）+ R4 P1-4（维持，锁序部分）；怀疑者 #33 采纳 | 新增一个设置 = 4 文件约 10 处改动 + 2 个手工同步点，漏一处即静默失效；`direct_hotkeys → clip` 嵌套锁顺序无守卫表达 | 阶段一（Rust 内聚）：每设置的「装载+落库+影子同步」收进 owner 模块三函数（load_from/apply/persist），setup/save_settings/托盘三写者收敛调同一函数（镜像 10→约 5）；direct_hotkeys 锁作用域收紧（register/DB 写移出临界区）+ 两锁获取点加「锁序：direct_hotkeys → clip，勿反向」注释；阶段二轻方案：DTO↔AppSettings 键集合锁定 vitest（不引 specta） | M | 与 P1-17 步 1/5 衔接（可先做，拆分时搬现成的） |
| **P1-19** | App.tsx vault 域动作抽离 | R4 P1-6（维持）；怀疑者 #36 采纳（工程侧） | 约 300 行 vault 域动作栖身 App 根而非 vault 插件模块，actions 装配依赖数组是 stale-closure 温床 | 提为 `plugins/vault/useVaultActions.ts`（入参 invoke 包装 + refreshVault 回调，返回与现 actions 等同接口），App.tsx 降为装配层；键盘栈不动（等第二域出现再抽象） | S | 无 |
| **P1-20** | 更新检查仅启动一次 | R9 P1-2（维持）；怀疑者 #25 采纳 | 常驻托盘应用启动时静默 check 一次，数月不提示更新 | useUpdater 加 24h 间隔 setInterval 重查（checkForUpdate 已幂等）；可选面板呼出超 N 天补查；~10 行，无遥测红线问题 | S | 无 |

### P2（18 条）

| 编号 | 标题 | 来源 | 一句话问题 | 修复摘要 | 量级 |
|---|---|---|---|---|---|
| **P2-01** | 呼出链路诊断埋点 | R1 建议4 + R5 建议4（#28 采纳） | 「呼出到可交互」无基线数据，P2-02 的收益判断无从谈起 | `toggle_panel`（show→emit）与前端 `zap://shown`（收到→列表就绪）两端 diag 时间戳 | S |
| **P2-02** | 索引扫描后台化 | R1 P2-4（#30 采纳；原报告自评置信度中，见 §4-4） | `AppIndex::scan`（目录树+UWP COM+图标缓存读）同步跑在 setup，呼出热键注册排后，冷启动「进程已起热键不可用」 | scan 挪后台线程，entries 空表初始化，扫完走 refresh 热替换 + `zap://index-ready`；icon backfill 挂扫描完成回调；接受极早期呼出应用源空窗 | S |
| **P2-03** | Everything 状态对用户零可见 + 端口探测可劫持 | 怀疑者新发现2 + R5 P2-1 | 文件搜索静默死亡时用户无法区分「没跑/端口被占/被停用」；probe 判定任何 HTTP 响应即缓存端口、JSON 失败不清缓存 | 设置页 Everything 区加状态行（复用现成 `everything_status` 命令，零新后端）+ 可选「重新检测」；probe 加「合法 JSON 且含 results 键」判定，search 的 JSON 失败路径同样 `PORT.take()`+`mark_failed()` | S |
| **P2-04** | Everything 下载无完整性校验即提权 | R3 P2-5 + R9 P2-4（合并） | 下载 zip → 解压 → UAC 提权全链路无哈希/签名校验 | 解压后提权前对 Everything.exe 做 Authenticode（WinVerifyTrust 验 voidtools）或维护 SHA256 表（未知版本拒绝自动执行、提示手动装）；真机确认官方是否签名后二选一 | S |
| **P2-05** | 供应链零审计 + actions 未固定 SHA | R9 P2-3 + R3 P2-6 | 无 cargo/pnpm audit、无 dependabot；五个 action 引用浮动 tag 且签名私钥同 runner | `.github/dependabot.yml`（npm/cargo/actions weekly grouped）+ CI 加 rustsec/audit-check（continue-on-error）+ `pnpm audit --prod`；五 action 固定 commit SHA；可选 release 加 environment 手动门 | S |
| **P2-06** | release 非幂等 + 工具链浮动 | R9 P2-1 + P2-6 | 资产上传中途失败成「半发版」（latest.json 缺失全体用户更新静默失败）；rust-toolchain 浮动 + 无手动补发通道 | `gh release create` 与 `gh release upload --clobber` 拆两步 + `upload-artifact` 留档；`src-tauri/rust-toolchain.toml` 钉 minor；release.yml 加 `workflow_dispatch` | S |
| **P2-07** | 卸载残留：明文历史 + 自启死链 | R3 P2-7 + R9 P2-5（合并） | 卸载不清 AppData（含明文剪贴板历史）与 HKCU Run 键 | 轻方案：`deleteAppDataOnUninstall: true` + release-notes「卸载将删除本地数据（含账号库），请先导出备份」；或保守：README 数据位置节明示手动清理；`installerHooks` PREUNINSTALL 清 Run 键（与 P2-09 同文件，边际≈0，真机验证后做） | S |
| **P2-08** | 安全卫生三项 | R3 P2-4 + P2-9 | open_item settings 分支 target 拼进 `cmd /C start` 无白名单；capabilities 非最小权限（opener 挂三窗口、global-shortcut/autostart 疑似未用） | settings target 白名单校验（settings.rs 静态枚举）+ 修正 Run 对话框注释；capabilities 裁剪（default.json 去 stagedrop/superpanel，desktop.json 裁剪验证后做）+ 功能回归一遍 | S |
| **P2-09** | 更新收尾残留（修法待仲裁） | R9 P0-1 改写降 P2；怀疑者 #25 采纳部分重叠 → **§4-2** | ready 态在 Windows 是死 UI、消失→重启空窗无解释、catch 把安装器启动失败误报为下载失败 | ready 分支文案改「更新将自动完成，zap 即将重启」级预期管理 + downloadUpdate catch 区分「下载失败/安装启动」+ 补完成反馈；installerHooks POSTINSTALL 拉起**不做**（除非真机证伪 /R 自动重启）；24h 重查已拆入 P1-20 | S |
| **P2-10** | 跨窗口主题一致 + 浮层掩码对齐 | R7 P2-6 + P2-7（#24 采纳） | 浅色主题不覆盖落区条/超级面板（两窗口独立文档无 appearance）；Alt+4 查看浮层无 30s 自动掩码与详情页不一致 | main.tsx 按窗口 label 挂载时读设置 applyAppearance（或刻意保留暗色则在设置描述写明，消「bug 感」）；VaultRevealOverlay 复用 30s 掩码计时 + 文案同步修正 | S |
| **P2-11** | 渲染顺手项：key 去索引 + 琥珀 token 化（memo 可选） | R2 F6/F7 + R1 P2-5 | `key=id+String(i)` 重排即整行重挂闪帧；7 处硬编码琥珀绕过 token；每击键重复携带图标 data URL | key 改 `kind:id`；7 处换 `var(--primary)/--zap-err`（或新增 `--zap-brand`）；前端按 id 建 Map 图标缓存；ResultRow memo 为可选（收益随 clipMax 增长） | S |
| **P2-12** | 测试补齐三件套 | R2 F10（#31 采纳） | usePanelSource/clampMenuPos（三处手工同步常量）/codeToKeyName 零覆盖 | 三个纯函数各补 ≤6 条断言（既有 mock 模板内增量）；merge 键盘面若按 P1-01 并入 View，顺手提纯按键决策 | S |
| **P2-13** | stagedrop 小步演进 + SearchEngine 锁纪律文档 | R4 P2-7 + P2-8 | 外部写者已由 P1-17 步 0 收口；BAR_SHOWN 仍在状态机外；clip 锁内可为/不可为只写在注释里 | `BAR_SHOWN` 并入 STATE（读侧改短暂锁，80ms 节奏无竞争顾虑；POSITION/ENABLE 保留原子量）；mod.rs:342 锁纪律注释升级为模块级文档；SearchEngine 方法数止损线成文 | S |
| **P2-14** | autostart 导出语义不成立 | R8 P2-7 | autostart 导入后无生效路径（save_settings 不消费，走独立注册表命令） | 导出时去掉 autostart 字段（诚实），或导入后前端检测差异提示手动开 | S |
| **P2-15** | 后台线程生命周期收口 | R4 P2-10（半边；paste 编排收口挂 P1-17 步 3） | register_drop_target 60s 永久重试无退出条件，无界生命周期线程 | 累计约 10 分钟后停并落 diag「drop target 注册放弃」；可挂 P1-10 日志批同文件顺路 | S |
| **P2-16** | 发布前手工冒烟 checklist 落档 | R5 P2-2 建议④ + R5 诚实清单 | drop→面板全链路、热键四槽、睡眠唤醒、多显示器/DPI 变更全靠口头纪律 | 把已踩坑（SHGetFileInfoW 并发/焦点 IME/DIBV5）的回归测试补到任何环境可重跑；手工项落成 checklist 随 release 走 | S |
| **P2-17** | vault 列表复制反馈口径不一致 | R7 P2-10（怀疑者对抗5：折中可取） | 列表复制=整列表替换 ✓+收面板，详情页=行内 ✓+不收，自动粘贴关闭用户连续抄取被打断 | 折中版：自动粘贴关闭时列表复制不收面板（该设置下用户已选择多步流）；「复制即走」主语义保留（可能是刻意拍板，注释未证） | S |
| **P2-18** | SuperPanel 动作图标语义失真 | R7 P2-8 | 所有动作行尾统一渲染 Copy 图标，搜索/进主面板类动作语义失真 | 图标随动作 id 分流（前端一份 id→图标映射与后端清单同源维护）；嫌耦合则非复制类不渲染行尾图标 | S |

**不立项记录**（维持现状 / 记为已知特性，防止重复报告）：R1 P2-6（每击键 scoped 线程——刻意设计量级可接受）、R1 P2-7（winsys 快照锁——可接受）、R1 P2-10（三窗口常驻内存——已知内存特性，权衡后不动）、R8 P2-5 前半（vault 删除二次确认——已存在，见 §4-3）、R10 提案 8/9（驳回）、R10 不推荐区四条（负向维持）、怀疑者驳回的欢迎条与 portable zip（降级挂起）。

---

## 2. 分阶段路线图

排期总原则：**P0 先行 → P1（先用户可感、后结构）→ P2 → 功能**；每笔提交可独立验证、可独立 revert；策略类判定随拆分提纯进纯函数（决策提纯纪律）；并行边界不动既有成本模型切分；全部改动尊重单人维护节奏（优先收敛不新增模式）。

### 2.1 「一次改动带两个收益」的合并机会（本路线图的主要结构）

1. **panic hook 同时收 P0-1 可诊断性与 R4 P1-3 残余**：A1 提交里 hook（P0）+ 主线程 4 处 expect 改 poison-tolerant（P2 风格统一）同批，验证手段相同（人为 panic 一把验）。
2. **设置页「关于」区一区四鸟**：版本号（修 SECURITY.md 悬空引用 R6 P2-10）+ 诊断出口（R6 P1-3/R9 P1-3）+ 1.0 叙事的家（新发现1）+ 超级面板露出文案（R6 P2-7）。
3. **user_version 迁移器是 P1-08 物化列的前置**：C1 先行，C2 的 nl_count/head 回填作为迁移步骤 3 顺路完成；未来软删等演进同走此轨。
4. **存储收口一个 PR 收五条**：图片预算（R1 P1-2）+ 文本上限（R8 P0-1）+ 锁时长（新-3）+ checkpoint/synchronous（R8 P2-4/R1 P2-8）+ clamp/RETURNING/substr 顺手项（R8 P2-1/P2-6/R1 P2-9）。
5. **键盘面 v2 一个 PR 收六处 + CI 防回归**：F1/F2/F3 主修 + F4/F5/F9 顺路 + R7 P2-5 footer 提示 + panel.tsx 契约注释 + grep 白名单。
6. **日志底座一次铺平三件事**：P1-10（stderr 迁移）与 P1-11（watcher 自愈）共用 diag 形态改造，且 P2-01 埋点、P2-15 重试上限（含 diag 行）搭同批。
7. **更新收尾一次发版验两件事**：C10 的 24h 重查 + 文案修正，随一次真实发版顺带完成 §4-2 的 /R 行为真机验证。
8. **超级面板恢复一个函数三缺陷**：P1-06 的 DIBV5 + 恢复守卫 + serde 卫兵同函数同批，共用真值表测试。

### 2.2 阶段 0：P0（专门批次，约 5-6 个工作日）

| 提交 | 内容与文件级落点 | 验证手段 | 对应条目 |
|---|---|---|---|
| A1 | panic hook 挂 `run()` 最前（src-tauri/src/lib.rs run / main.rs），钩子内 `diag::log`（providers/diag.rs 补进程生命周期调用）+ 保留 stderr；顺手 lib.rs:1473/1493/1710/1647 四处 `expect("...poisoned")` 改 `unwrap_or_else(into_inner)` | cargo test / clippy / fmt + 真机：人为 panic 验证不重复 panic 且 zap.log 有痕 | P0-1①④ |
| A2 | open_db 失败降级：providers/clipboard.rs（open_db/quick_check）、providers/mod.rs:179（expect 改降级调用）；坏库改名逻辑 + diag；前端提示（src/App.tsx toast 或 src/views/SettingsView.tsx 备份区指引）；新增纯 Rust 损坏用例（临时文件逐字节破坏） | cargo test 新增损坏用例 + 真机：预置坏库启动冒烟 | P0-1② |
| A3 | vault 备份提醒：src/views/SettingsView.tsx 备份区（上次导出时间 + 30 天提醒）+ 后端设置位（providers/vault.rs 或 appconfig） | vitest + 真机 | P0-1③ |
| A4 | release.yml（.github/workflows）附加 SHA256 产物步骤；winget-pkgs 清单 PR（外部）；微软误报提交（外部）；证书渠道调研备忘 | 推 tag 检查 notes 与三件套；winget PR 过审 | P0-2 步 1-3 |
| A5 | 首启自动呼出（lib.rs setup 读 `first_run_done` + toggle_panel）；托盘 tooltip 动态化（lib.rs:1967 与热键注册失败分支 lib.rs:1854）；空态死文案删除（src/App.tsx:1277）；超级面板设置文案（SettingsView.tsx） | vitest + 真机三态：首启自动呼出 / 二启不再 / 模拟热键占用 tooltip 告警 | P0-3 + P1-10 热键两点 |

### 2.3 阶段 1：P1 用户可感（专门批次，约 8-10 个工作日）

| 提交 | 内容与文件级落点 | 验证手段 | 对应条目 |
|---|---|---|---|
| B1 | 超级面板恢复三修：providers/superpanel.rs:61-111（DIBV5 换 set_image_dib + 恢复守卫 + 耗时 diag）+ :186（MAX_INPUT_BYTES 卫兵） | 真值表单测 + 真机：大图恢复 / 文件选区触发 / MB 级选文 | P1-06 |
| B2 | Ctrl+V 常驻监听：src/plugins/panel.tsx:50-70（awaitPaste 语义重设计）+ src/plugins/json/JsonWorkbench.tsx + text/ToolWorkbench.tsx（按钮降级等待指示） | 真机：入口词进入 → 空态按 Ctrl+V 即填 | P1-02 |
| B3 | 键盘面 v2：src/hooks/useEscCapture.ts（isComposing）、src/App.tsx:812-850（merge 守卫或并入 View:60-70）、:1033-1042/:1239（ctxMenu 分支迁 window 捕获）、:1283（shown 清 ctxMenu）、src/plugins/super/SuperPanel.tsx:68（capture+卫兵）、src/plugins/panel.tsx:6-7（契约注释）、CI onKeyDown 白名单脚本 | vitest + 真机：右键菜单全键盘 / 合并模式进设置页 / IME 合成期 Esc / 点行后导航 | P1-01 |
| B4 | 主线程写路径：src-tauri/src/lib.rs:1108-1205（clip 两段式）、:1115-1119（record_app_launch 挪 spawn_blocking）、:347-581（save_settings 单事务）、providers/clipboard.rs open_db（busy_timeout=1500）、providers/stage.rs:94-135（commit_add 事务） | cargo test（行号契约测试不变）+ 真机：MB 级 clip 回贴 | P1-07 |
| B5 | 密码防护与补救：providers/clipboard.rs watcher 入库比对（:191-200 区域）+ providers/vault.rs（解密缓存与失效钩子）+ lib.rs（新增 clip_delete/clip_clear_unpinned 命令注册）+ src/plugins/registry.tsx（clip 右键）+ src/App.tsx（关闭开关时的清空动作）+ vault_import 清理（vault.rs） | cargo test（比对/清理等值语义）+ 真机：浏览器 PM 复制密码不入库、右键删除/清空 | P1-09 |
| B6 | 受保护复制统一：providers/vault.rs:281/322、providers/superpanel.rs:254-260 改 copy_text_private；lib.rs:1571-1594 焚除窗口跳过 | cargo test + 真机：复制备注后 Win+V 无痕、焚除期内快速粘贴回退 | P1-13 |
| B7 | Everything 认证：providers/everything.rs:162-166/188-194（随机凭证 + seed_ini 只补缺）+ providers/files.rs:42/89-92（Basic Auth） | cargo test（ini 改写逻辑）+ 真机：Everything 查询回归、手动关 HTTP 不被复活 | P1-12 |
| B8a | 日志底座：providers/diag.rs（级别前缀/启动行）、lib.rs:1854/1884/1895/1674、clipboard.rs 五处、everything.rs:330-342、files.rs:94-106 eprintln→diag；hotkey_capture.rs capture.log 并入 | cargo test + 真机看 zap.log 四问可答 | P1-10 |
| B8b | watcher 自愈：providers/clipboard.rs:240-331（respawn + 退避 + 存活日志） | 真机：结束监听窗口/杀 arboard 场景验证复活并留日志 | P1-11 |

### 2.4 阶段 2：P1 结构与流程（约 12-15 个工作日；拆分可随日常开发逐步推进）

| 提交 | 内容与文件级落点 | 验证手段 | 对应条目 |
|---|---|---|---|
| C1 | 迁移器：providers/clipboard.rs open_db（PRAGMA user_version + BEGIN IMMEDIATE + 具名迁移函数）；收编吞错 ALTER 为步骤 1/2 | cargo test 迁移幂等/并发用例 | P1-14 |
| C2 | 存储收口（**依赖 C1**）：clipboard.rs/clipimage.rs（文本上限、nl_count/head 物化列 = 迁移步骤 3、synchronous=NORMAL、退出 checkpoint、RETURNING）、appconfig.rs（图片预算键）、lib.rs（clip_max clamp + DTO）、SettingsView.tsx（双阈值设置与文案）、mod.rs:693-699（latest substr 版） | cargo test（含旧库升级用例）+ 真机旧库启动 | P1-08 |
| C3 | 设置链路阶段一：providers/appconfig.rs 与各 owner 模块（load_from/apply/persist）、lib.rs setup/save_settings/托盘改调 owner、lib.rs:444 锁作用域收紧 + 锁序注释（lib.rs direct_hotkeys 与 mod.rs:165 clip 两处） | cargo test + 设置保存/冲突回滚/配置导入导出往返 | P1-18 |
| C4 | 拆分五步（每步一笔提交）：步0 解反向边（providers/stagedrop.rs:889、superpanel.rs:279、lib.rs:90）→ 步1 settings.rs（lib.rs:225-664）→ 步2 state.rs → 步3 panel.rs + should_hide_on_blur 等三纯函数提纯 + 单测 → 步4 commands.rs + helper | 每步 cargo test/clippy/fmt/tsc/vitest；步0 真机拖放两路径冒烟；步3 真机三失焦场景 | P1-17 |
| C5 | vault 动作抽离：src/plugins/vault/useVaultActions.ts + src/App.tsx:492-1020 收编 | vitest（dispatch 测试不动）| P1-19 |
| C6 | 设置脏态保护：src/views/SettingsView.tsx:499-513（脏标记 + zap-dialog 确认条 + 保存按钮禁用态）+ src/App.tsx:232-234（refreshAppearance 改从 settings 退出） | 真机：改热键后 Esc 弹确认 / 无更改时按钮置灰 | P1-03 |
| C7 | vault 鼠标面：src/plugins/registry.tsx:114-121（ctxActions 五项）+ src/plugins/vault/VaultDetailView.tsx:174-236（网址「打开」按钮） | 真机：右键菜单全动作、打开网址 | P1-04 |
| C8 | 合并模式收敛：src/App.tsx:929-947（非 clip 禁用态）、:1443-1451（chip 条件显示）、:1500-1508（Esc 提示三元） | 真机：合并模式点文件行无效、提示随模式切换 | P1-05 |
| C9 | 「关于」区：src/views/SettingsView.tsx 新分区（版本/打开数据文件夹/日志说明）+ SECURITY.md:14、README.md:131、.github/ISSUE_TEMPLATE/bug_report.yml | 真机：按钮 reveal 数据目录 | P1-16 |
| C10 | 更新 24h 重查 + 收尾文案：src/views/useUpdater.ts:49-58（setInterval）+ src/views/SettingsView.tsx:1246-1294（ready 文案/catch 区分）——**随一次真实发版顺带验证 /R 自动重启**（§4-2 的裁决输入） | 真机发版端到端一次 | P1-20 + P2-09 |
| C11 | CHANGELOG.md 回补 + release.yml per-tag notes + scripts/verify-version.mjs（接线 .githooks/pre-push、ci.yml、release.yml）+ CONTRIBUTING.md:47 + README 插件立场一句话 | 脚本本地跑通 + CI 绿 | P1-15 |

### 2.5 阶段 3：P2（约 8-10 个工作日集中；多数可顺路摊薄）

| 提交 | 内容与文件级落点 | 对应条目 |
|---|---|---|
| D1 | 呼出埋点：lib.rs toggle_panel + src/App.tsx zap://shown 处理器 diag 时间戳 | P2-01 |
| D2 | 索引后台化（**依赖 D1 基线**）：providers/apps.rs scan、providers/mod.rs:188-189、lib.rs setup | P2-02 |
| D3 | Everything 状态行 + probe 校验：SettingsView.tsx Everything 区、providers/files.rs:37-50/93-107 | P2-03 |
| D4 | 下载校验：providers/everything.rs:72-98（Authenticode 或 SHA256 表） | P2-04 |
| D5 | 供应链：.github/dependabot.yml、ci.yml audit 步骤、release.yml 五 action 固定 SHA | P2-05 |
| D6 | release 幂等 + 钉版：release.yml（create/upload 拆分 + upload-artifact + workflow_dispatch）、src-tauri/rust-toolchain.toml | P2-06 |
| D7 | 卸载：tauri.conf.json（deleteAppDataOnUninstall）+ .github/release-notes.md 文案 + installerHooks PREUNINSTALL（真机验证后） | P2-07 |
| D8 | 卫生三项：lib.rs:1150-1155（settings 白名单）、src-tauri/capabilities/default.json + desktop.json 裁剪 + 功能回归 | P2-08 |
| D9 | 主题/掩码：src/main.tsx（按 label applyAppearance）、src/plugins/vault/VaultRevealOverlay.tsx（30s 掩码） | P2-10 |
| D10 | 渲染顺手：src/App.tsx key 改 kind:id、tokens 7 处替换（panel.css/results.css/stage.css/imgpreview.css）、图标 Map 缓存 | P2-11 |
| D11 | 测试三件套：src/plugins/panel.tsx / src/lib/ctxMenu.ts / SettingsView.tsx codeToKeyName 提取导出 + vitest | P2-12 |
| D12 | stagedrop 小步：providers/stagedrop.rs BAR_SHOWN 并入 STATE + mod.rs:342 锁纪律模块文档 | P2-13 |
| D13 | autostart 语义：lib.rs:611 / appconfig.rs / configfile.ts（去字段或导入提示） | P2-14 |
| D14 | 线程生命周期：providers/stagedrop.rs:688-696 重试上限 + diag（可挂 B8a 顺路）；paste 编排收口挂 C4 步3 | P2-15 |
| D15 | 冒烟 checklist：docs/checklist（release 随行）+ 补 SHGetFileInfoW/焦点 IME/DIBV5 回归到环境无关 | P2-16 |
| D16 | vault 复制反馈折中：src/App.tsx:546-557（自动粘贴关闭时不收面板） | P2-17 |
| D17 | SuperPanel 图标分流：src/plugins/super/SuperPanel.tsx:93-97 | P2-18 |

### 2.6 阶段 4：功能（按产能插入，见 §3）

---

## 3. 功能与创新幸存清单

### 3.1 采纳（进入路线图）

| 提案 | 裁决 | 最小切片 | 挂载点 |
|---|---|---|---|
| **行内计算器 v1**（R6 P1-4 + R10 提案1 合并版，怀疑者对抗1：round2 功能类最高优先级） | 采纳（合并版） | `providers/calc.rs` 实现 `Plugin::probe`：四则/括号/百分比纯函数解析器（禁函数调用禁变量，「含运算符才算」准入防误识别），产出 kind=`calc` 条目（title=算式 subtitle=结果），activate=复制；解析器+单测一天级。**v2 单位换算**（`100 km 到 mi` 静态单位表）独立切片再加两天，不拖 v1 | probe 骨架（mod.rs:583-592）第三个消费者；**kind 必须注册进 src/plugins/registry.tsx**（未注册 kind 落 dispatch.ts external 的已知陷阱）；meta 参照 texttools.rs:64-74 |
| **URL 追踪参数清洗**（R10 提案2，怀疑者 #2 采纳收窄） | 采纳（收窄） | v1 范围锁死：`recognize` 加 is_url_with_tracking（`utm_*` 通配 + 高频国内参数表），detect 给内联「清洗后链接」答案（复制即走）；工作台「去追踪参数/去零宽字符/全角引号」三按钮随批；两天级，参数表用户不可编辑 | texttools.rs:137-244 recognize 既有判定模式 + ToolBundle；内联答案有测试锁定先例（texttools.rs:471） |
| **Everything 状态行**（怀疑者新发现2） | 采纳 | 设置页状态行复用现成 `everything_status` 命令（零新后端）+ 可选「重新检测」 | SettingsView.tsx Everything 区；与 P2-03 同批 |
| **空态死文案删除 / 超级面板露出文案 / 首启 tooltip** | 采纳/改写 | 已并入 P0-3（阶段 0） | App.tsx:1277、lib.rs 托盘 tooltip、SettingsView 文案 |
| **诊断出口「关于」区**（三方收敛 + 新发现1 改写形态） | 采纳（改写形态） | 已并入 P1-16（阶段 2） | SettingsView 新分区 |

### 3.2 改期 / 挂起（保留触发条件，不进当前路线图）

| 提案 | 裁决 | 触发条件 / 复活路径 |
|---|---|---|
| 窗口贴靠（右键四向，R10 提案4） | 降级改期 | 核心缺口（计算器/清洗/诊断/更新）清完之后；前置：DPI/最大化恢复语义真机收口；切片照 R10（sysutil snap_window + secondary_action 分支 + ctxActions 两项） |
| zap:// 深链协议（R10 提案5） | 降级 12 个月+ | 出现第一个真实集成需求（文档/社区/消费方三者居其一）；三闸（长度上限/动词白名单/默认关）设计保留 |
| 片段模板库（R10 提案6） | 降级 12 个月 | v1/v2（占位符）绑定立项防半成品；v1 与固定剪贴板的重叠已被两轮确认 |
| 数据插件包（R10 提案7） | 降级 12 个月 | 唯一可先行半件 = config_import「分段导入预览」（vault 恢复预览 UX 先例 mod.rs:465-480）；minisign 签名砍 |
| 「此刻」时间桶预测（R10 提案3） | 降级整条挂起（连埋点也不做） | 出现真实用户反馈「空面板顺序不懂我」时复活 R10 的双闸门设计（观测不足回退 + 只做加法槽位） |
| portable zip | 降级改期 | winget 验证有效且国内绿软接入价值确认后重议 |

### 3.3 驳回与负向维持（防重复立项）

- **带负载启动**（R10 提案8）驳回：触碰「快捷命令」拍板且翻案论证不成立（注入时序正是简报点名脆弱区；省的只是最后一次 Ctrl+V）。
- **局域网接力**（R10 提案9）驳回挂 36 个月观察：唯一开网络端口的提案，估工偏乐观；三闸底线保留。
- **欢迎条**驳回（tooltip 等价承载、零状态机）；**vault 二次确认**驳回（现状已满足，见 §4-3）。
- 负向维持：语义 embedding / 动态插件 / 宏录制 / 前台轮询（R10 数值化重启条件照单全收，是本轮最佳实践范式）。

---

## 4. 遗留分歧点（交仲裁，5 条）

1. **R7 P0-1（Ctrl+V 错位）级别：红队降 P1 vs 怀疑者维持 P0。** 红队口径：同句话给了可用鼠标出路、2 秒恢复零数据后果，容不进本项目「进程死/数据丢/无法自救」的 P0 桶；怀疑者口径：入口词进入的默认落点 + 新用户第一条引导文案为假，第一印象级事故。**倾向：按红队记 P1（口径一致性优先，P0 桶失守会让后续定级失锚），但排阶段 1 首批**（S 量级，无需为定级推迟修复）。若仲裁更重产品叙事，升 P0 不影响排期。
2. **R9 P0-1（更新收尾）修法：红队改写降 P2 vs 怀疑者 #25 采纳（含 installerHooks）。** 事实分歧核心：怀疑者沿用 R9 的「更新把 zap 搞没了」，红队从 registry 源码证实 tauri-plugin-updater 2.14.0 默认 `restart_after_install=true` + Passive 模式追加 `/R`（updater.rs:226、config.rs:21-22/53-57），装完自动重启。**倾向：采红队事实（证据链三级实证 vs 怀疑者未下钻插件源）**；残留按 P2-09 执行（文案 + catch 区分 + 完成反馈），installerHooks 不做；24h 重查独立维持 P1（P1-20，红队全对）。裁决输入：C10 随真实发版真机验证一次 `/R` 是否确实生效。
3. **R8 P2-5（vault 删除无二次确认）：红队「P2 全部维持」vs 怀疑者 #35 驳回。** 怀疑者实证 VaultDetailView.tsx:288-319 确认弹窗已存在；R8 诚实清单自认「前端 UI 未逐一核验」。**倾向：采怀疑者驳回**（R8 的主张建立在未核验前提上，怀疑者已补上该证据）；「软删/30 天撤销」降级为 12 个月可选加分项（数据层实现可走 P1-14 迁移器）。
4. **P2-02（索引扫描后台化）：怀疑者直接采纳 vs R1 原文自评置信度中且建议「先按建议 4 加埋点拿基线」。** 两份裁决引用同一事实但投入判断冲突：启动扫描耗时未实测（warm 缓存可能仅几十 ms）。**倾向：折中执行——D1 埋点先行（P2-01），D2 后台化随后**（怀疑者已接受「极早期呼出应用源空窗」的产品代价，方案本身低风险；只是顺序尊重原报告的证据纪律）。
5. **P1-01 中 F3（useEscCapture IME 卫兵）的证据强度：红队维持 P1 基于团队卫兵先例旁证 vs 事件形态未真机验证（若 WebView2 合成期 Esc 的 key="Process" 则现状已被 key 判断挡住）。** **倾向：照修不照争**——一行防御性对齐与既有两处卫兵口径统一，零成本锁死各输入法差异；真机若证实 key=Process，在提交说明标注「防御性对齐」即可，不改排期不影响级别。

**备忘（不计入分歧，不影响裁决）**：Rust 测试计数三处口径打架（简报 117+1 / R4 数 120 / 提交 69b3c5b 自述 119+1）、vitest 31 vs 简报 32——实施时以实跑为准，README 已按 P1-15 去硬编码；R6 P0-2 原文「呼出热键露出是设置页与底栏」与怀疑者核验「仅设置页」矛盾，怀疑者已裁决以核验为准，本报告采信怀疑者。

---

## 5. 总工作量评估

| 阶段 | 内容 | 量级合计 | 执行方式 |
|---|---|---|---|
| 阶段 0（P0） | panic/DB 软着陆 + 分发可信首步 + 首启引导 | 5 提交 ≈ 5-6 天 | **专门批次**（每笔提交独立验证，A4 含外部流程并行发起） |
| 阶段 1（P1 用户可感） | 超级面板/Ctrl+V/键盘面/写路径/密码防护/复制通道/Everything/watcher+日志 | 9 提交 ≈ 8-10 天 | **专门批次**（可拆两个冲刺：B1-B3 交互周、B4-B8 数据与日志周） |
| 阶段 2（P1 结构与流程） | 迁移器 + 存储收口 + 设置链路 + 拆分五步 + 前端结构 + 设置脏态/vault 鼠标/合并模式/关于区/更新/CHANGELOG | 11 提交 ≈ 12-15 天 | 混合：**C1+C2、C3、C4 拆分为专门批次**（C4 每步独立提交可随日常开发逐周推进）；C5-C11 均为 S-M 可随日常顺路 |
| 阶段 3（P2） | 18 个 S 项 | ≈ 8-10 天集中 / 顺路摊薄后 ≈ 3-4 天净增 | **以顺路为主**：D10/D13/D14/D16/D17 挂同文件批次搭车；D1-D9 可攒一个「发布工程周」一次清完 |
| 阶段 4（功能） | 计算器 v1+v2、URL 清洗 | ≈ 4-5 天 | 产能盈余时插入；计算器不依赖任何前序项，可提至阶段 1 之后随时做 |

**合计 ≈ 38-46 个工作日（单人满负荷），按单人日常节奏现实为 2-3 个月自然消化。**

- **必须专门批次的**：A 批全部（P0）、B4+B5（写路径与密码防护，触同一锁域）、C1+C2（迁移器+存储收口强耦合）、C4（拆分五步——唯一 L 项，但每步可独立回滚）、C3（设置链路收口）。
- **可随日常开发顺路的**：F7 琥珀 token、F8 refreshAppearance、pin RETURNING、autostart 语义、stagedrop BAR_SHOWN、spawn_blocking helper、register_drop_target 重试上限、README 立场一句话、文档漂移修正、vault 复制反馈折中、SuperPanel 图标——这些 S 项都在更大提交触碰的同一文件里，搭车完成即可。
- **外部流程（不占代码产能但需发起）**：winget 清单 PR、微软误报提交、证书渠道调研、一次真实发版（验证 /R 与更新闭环）。
- **给最终仲裁的排序建议**：若产能只能保一段，保阶段 0 全部 + 阶段 1 的 B1/B3/B4/B5/B8——这九笔提交覆盖了全部「不可自救」后果与最高频故障的定位能力，是「1000 陌生用户 + 事故远程诊断」标准的最低防线。
