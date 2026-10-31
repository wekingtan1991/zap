# 09 · 发布与运维维度评审（round1，2026-10-29）

评审人领地：CI/release 流水线、代码签名缺失后果、更新器健壮性、NSIS 打包、崩溃诊断通道、版本纪律、依赖供应链、Windows 支持面。
事实源：磁盘当前状态（含简报所述 5 文件在途改动）。已读简报 00-brief.md，尊重拍板 3/4/5（无遥测、国内双源暂不做、本地优先）。

---

## 1. 角色结论摘要

1. **交付链路骨架是健康的**：tag↔tauri.conf.json 有 CI 强校验（release.yml:18-27）、三件套（setup.exe/.sig/latest.json）生成与上传齐全、latest.json 用 `$(cat)` 处理签名尾部换行的细节注释在案（release.yml:68-69）、「已存在 Release 绝不覆盖」的纪律明确（release.yml:88-89）、release-notes 已有 SmartScreen 提示文案（.github/release-notes.md:3-4）。minisign 公钥内嵌客户端、私钥丢失后果已文档化（CONTRIBUTING.md:49-57）。
2. **最大缺口在「更新收尾」**：Tauri 官方文档明确 Windows 上执行安装步骤时应用即被退出、且不会自动重启（v2.tauri.app/plugin/updater/："the application is automatically exited when the install step is executed"）。useUpdater 的 `ready` 状态与「重启以完成更新」按钮（useUpdater.ts:108、SettingsView.tsx:1246-1294）在 Windows 上不可达——用户视角是「点了下载安装，进度到顶后 zap 直接消失，无任何解释」。这是必现路径，装机量上去后会是第一客诉源。
3. **无签名 × 本应用的高启发式特征组合是真实风险**：SendInput 键盘注入（superpanel.rs:157、winsys.rs:201）+ DPAPI 加解密（dpapi.rs:34/76）+ 剪贴板常驻监听（clipboard.rs:5）+ HKCU Run 自启（lib.rs:1638/1998-2004）+ 下载 zip 并提权执行（everything.rs:82-86）——这套行为特征与信息窃取类样本的启发式画像高度重合，Defender 误报概率不可忽视。个人开发者有零成本/低成本的缓解路径（见 P1-1）。
4. **诊断通道有底子但用户不可达**：`zap.log`（1MB 轮转、写失败静默，diag.rs:14-38）存在且设计克制；但 panic 无 hook 不落盘（全仓无 `set_hook`）、release 构建 stderr 不可见（main.rs:2，`eprintln` 五处全白写）、设置页/托盘无「打开日志」入口、bug 模板不引导取日志——出事后依赖用户自己摸到 `%APPDATA%\com.ais.zap\`。
5. **流程性手工活已有漏账实锤**：README 测试数停在 113（README.md:131）而简报记录已是 117；CONTRIBUTING 说「版本号同步三处」却只列两处（CONTRIBUTING.md:47，实际四处含 package.json/Cargo.lock）；release 校验只盯 tauri.conf.json。一个 ~15 行脚本可全部收口。

**P0 × 1，P1 × 3，P2 × 6。**

---

## 2. 发现清单（P0 > P1 > P2）

### P0-1 Windows 更新安装即退出应用，前端收尾态不可达，用户感知为「更新把 zap 搞没了」

- **证据**：
  - `src/views/useUpdater.ts:96-108`：`await u.downloadAndInstall(...)` 后 `setUpdateStatus("ready")`；
  - `src/views/SettingsView.tsx:1246-1250`：`ready` 态文案「更新已就绪，重启 zap 完成安装」；`:1285-1294`：「重启以完成更新」按钮（`relaunch()`）；
  - Tauri v2 官方文档（v2.tauri.app/plugin/updater/）：*"On Windows the application is automatically exited when the install step is executed due to a limitation of Windows installers"*，且无自动重启；
  - `src-tauri/tauri.conf.json:62-64`：NSIS 为唯一打包目标，此路径就是全部 Windows 用户路径；
  - `useUpdater.ts:92` 的 `set_hold_panel`（lib.rs:1707-1709 配套）只能保住下载/UAC 阶段的面板，保不住安装阶段的进程退出。
- **生产后果**：用户点击「下载并安装」→ 进度条走完 → 进程被安装器杀掉、面板消失、无提示。普通用户的第一反应是崩溃/数据丢失，第二个反应是去 GitHub 开 issue。升级实际成功了，但交付体验断在最后一米，且没有任何「更新完成」的正反馈。
- **具体建议与权衡**：
  1. 用 Tauri 的 NSIS `installerHooks`（`bundle.windows.nsis.installerHooks`）在 `NSIS_HOOK_POSTINSTALL` 里 `Exec` 新版主程序，实现「装完自动拉起」；权衡：全新安装也会自动启动（对启动器类应用反而合理）；API 细节需真机验证一次（见诚实清单）。
  2. 同步修正前端文案：Windows 下把 `ready` 分支改为「安装包已就绪，若 zap 自动关闭请重新打开」级别的预期管理文案，并在 `downloadUpdate` 的 catch 里区分「下载失败」与「安装启动」（后者不该显示为错误）。
  3. 若不想动 NSIS hook，最低成本方案：在 release-notes 与 README 自动更新一节写明「安装时 zap 会自动关闭，完成后请重新打开」。
  - 现实约束：全部改动 ≤ 50 行 + 一次发版验证，个人开发者一个下午可完成。
- **置信度**：高（文档引证明确，代码路径闭合）。hook 方案的变量细节为中。

### P1-1 无代码签名 × 高启发式行为特征：SmartScreen 拦截之外，Defender 误报是更实际的「装不上」

- **证据**：
  - 行为特征全部在案：`SendInput` 键盘注入（src-tauri/src/providers/superpanel.rs:157、winsys.rs:201）、DPAPI `CryptProtectData/CryptUnprotectData`（dpapi.rs:34/76）、剪贴板常驻监听（clipboard.rs:5 `clipboard_master::Master`，lib.rs:1786 启动）、HKCU Run 自启写注册表（lib.rs:1998-2004）、从 voidtools.com 下载 zip 并解压后单次 UAC 提权启动（everything.rs:82-86、288；README.md:21/54）；
  - 安装包无签名：`tauri.conf.json:62-73` bundle 配置无任何 signing 项；release-notes.md:3-4 自认「安装包未签名」；
  - Cargo.toml:62 `panic = "abort"` + strip（:63）——无 PDB 无符号，被误报后给微软的申诉材料也更薄。
- **生产后果**：SmartScreen（「更多信息→仍要运行」可过，且有文案兜底）只是第一层；Defender 启发式若判定为 infostealer 家族（DPAPI+剪贴板+键盘注入+自启的画像），会直接隔离 exe，用户连「仍要运行」的机会都没有；且每次发版哈希变化，SmartScreen 信誉清零重来。
- **具体建议与权衡**（个人开发者可执行序）：
  1. **零成本**：每次发版在 release notes 附安装包 SHA256（release.yml 里 `sha256sum` 生成、写入 notes），给用户自校验路径；
  2. **零成本**：向微软提交误报分析（security.microsoft.com 的「将文件提交给微软进行恶意软件分析」选「可能被错误检测」）——免费、是建立信誉的正路，尤其建议在每次大版本后主动提交一次；
  3. **低成本**：证书渠道按现实性排序——Certum 开源开发者代码签名证书（年费几十欧元级、对个人开源友好）→ SignPath Foundation（OSS 免费签名服务，需组织身份）→ Azure Trusted Signing（月费约 $10 但个人准入政策多变）。买不买是预算问题，但把三条渠道的现状核清楚是一次性的（见诚实清单）;
  4. **降启发式的编译/清单手段**：保持现有 strip+无壳现状（不要加壳）；Everything 的「下载→解压→UAC 执行」改为下载后先展示 SHA256 让用户确认再执行（同时消掉 P2-4）——「下载并提权执行」是启发式权重最高的单一行为；
  5. 发布页提示文案已有（release-notes.md:3-4），保持。
- **置信度**：高（行为特征为代码实证；Defender 是否实际拦截不可离线预测，见诚实清单）。

### P1-2 更新检查只在进程启动时做一次：常驻托盘应用可能数月不提示更新

- **证据**：`src/views/useUpdater.ts:49-58`：`useEffect(..., [applyUpdate])` 启动静默 `check()` 一次，失败 `catch(() => {})`；全文件无任何定时器/重试；托盘菜单无「检查更新」项（lib.rs:1930-1948 只有 show/设置/剪贴板/自启/退出）。
- **生产后果**：zap 是常驻托盘工具，用户的典型会话是「开机自启 + 数周不退出」。启动时检查一次 = 更新提示率约等于零；用户只有手动重启时才可能看到新版本。结合 P0-1，更新闭环的两端（提示端、安装端）都是断的。
- **具体建议与权衡**：加一个 24h 间隔的 `setInterval` 重查（复用 `checkForUpdate`，检查中态已幂等，useUpdater.ts:73）；可选：面板每次呼出超过 N 天未查则补查一次。无需新增权限与依赖，~10 行。代价：每 24h 一次对 `releases/latest/download/latest.json` 的 GET，无遥测红线问题（拍板 5 不受影响）。
- **置信度**：高。

### P1-3 崩溃诊断三缺：panic 不落盘、日志无用户入口、bug 模板不引导取日志

- **证据**：
  - 全仓 `src-tauri/src` 无 `set_hook`（findstr 全量无命中）；Cargo.toml:62 `panic = "abort"` → panic 即死，`zap.log` 无记录；
  - main.rs:2 `windows_subsystem = "windows"` → release 下 stderr 无处可去；lib.rs:1674、1854、1884、1895 等处的 `eprintln!` 诊断信息在发布版全部丢失（diag.rs:1-2 的模块注释自己也点明了这一点，但只有三个模块真正走了落盘）；
  - `diag::log` 覆盖面窄：仅 stage.rs:287、stagedrop.rs（多处）、vault.rs 焚除路径（vault.rs:100-134）——启动失败、热键注册失败、Everything 拉起失败等关键启动期事件全走 eprintln；
  - 无用户入口：SettingsView.tsx 全文无「日志/诊断」（findstr 无命中）、托盘菜单无（lib.rs:1930-1948）；
  - bug_report.yml:46-50 的「补充信息」不提示 zap.log 位置。
- **生产后果**：1000 个用户出现崩溃时，你收到的只有「打开没反应/闪退」，无法定位。而修复面其实很小：panic hook 20 行 + 一个托盘/设置入口 10 行 + 模板一句话。
- **具体建议与权衡**：
  1. `setup` 最前面挂 `std::panic::set_hook`，把 panic message + location 写入 `diag::log`（`panic=abort` 下 hook 仍会在 abort 前执行），同时把 lib.rs 各处启动期 `eprintln` 改为 `diag::log`（保持 stderr 双写）；
  2. 设置页「关于」区加「打开日志文件夹」——用现有 opener 插件 reveal `%APPDATA%\com.ais.zap`（README.md:112 已文档化该路径，缺的只是入口）；
  3. bug_report.yml 补充信息项加一句：「日志位于 %APPDATA%\com.ais.zap\zap.log（用记事本打开，抹去个人内容后粘贴）」。
  - 权衡：日志内容注意隐私红线（CONTRIBUTING.md:34：账号密码不得入日志）——现 diag 调用点均已遵守，新增调用点保持同口径即可。
- **置信度**：高。

### P2-1 release.yml 资产上传非幂等，失败后产物丢弃，需人工救场

- **证据**：release.yml:80-93 单条 `gh release create` 同时建 Release 并上传三件套；:88-89 注释明确「已存在即失败示警」。构建产物在 runner 的 `src-tauri/target/`（release.yml:65），无 `actions/upload-artifact` 留档；job 无 `workflow_dispatch` 触发器（release.yml:3-5）。
- **生产后果**：`gh release create` 先建 Release 后传资产，若 `.sig`/`latest.json` 上传中途失败，Release 已存在、重跑在 create 步失败——latest.json 缺失 = 全体存量用户的更新检查静默失败（useUpdater.ts:57 吞掉），形成「半发版」状态，只能手动删 Release 或补传。构建产物也随 runner 销毁而丢失。
- **具体建议与权衡**：拆两步——`gh release create "$TAG" --notes-file ... "$ART"` + `gh release upload --clobber "$TAG" "$ART.sig" latest.json`（create 保持「已存在即失败」语义不变，upload 幂等可重跑）；再加一步 `actions/upload-artifact@v4` 上传三件套作为灾备留档。约 10 行 yaml。
- **置信度**：高。

### P2-2 版本四处 bump + README 测试数为纯手工，漏账已发生

- **证据**：版本现值四处一致 0.13.0（package.json:4、tauri.conf.json:4、Cargo.toml:3、Cargo.lock:6092），但全靠手对：CONTRIBUTING.md:47 写「版本号需要同步三处」却只列两处（tauri.conf.json/Cargo.toml），package.json 与 Cargo.lock 未入册；release.yml:18-27 只校验 tauri.conf.json 一处；README.md:131「后端单元测试（113 个）」落后于简报记录的 117（00-brief.md:20）。
- **生产后果**：某次 bump 漏掉 package.json → Release 资产的 tauri.conf 版本正确、npm 侧版本漂移，前端 `@tauri-apps` 生态无碍但仓库元数据不一致渐积；README 测试数每次加测试都要人肉同步，已落后 4 个。tag 只校验一处意味着「四处不一致也能发出去」。
- **具体建议与权衡**：写一个 `scripts/verify-version.mjs`（读四处 JSON/TOML/lock 的 version 断言相等，~25 行，零依赖），三处接线：`pre-push` 钩子（.githooks/pre-push 末尾追加）、ci.yml 加一步、release.yml:18-27 的现有校验替换/加严；README 测试数改为不写死（「cargo test --lib」后注「数量以实际输出为准」）或由脚本校验。权衡：Cargo.lock 版本行号随依赖变动漂移，脚本用 cargo metadata 或简单文本匹配均可，后者够用。
- **置信度**：高。

### P2-3 供应链零审计：无 cargo audit、无 pnpm audit、无 dependabot/renovate

- **证据**：ci.yml/release.yml 全文无 audit 步骤；`.github/` 目录仅有 ISSUE_TEMPLATE、PULL_REQUEST_TEMPLATE.md、release-notes.md、workflows（无 dependabot.yml）；无 deny.toml（src-tauri 列表）；本机 `pnpm audit` 因 npmmirror 镜像无 audit 端点不可跑（ERR_PNPM_AUDIT_ENDPOINT_NOT_EXISTS，仓库无 .npmrc，为用户全局镜像配置）。
- **生产后果**：rusqlite、reqwest、windows-rs 等重依赖的安全通告（RustSec/ghsa）无人盯；一个带 CVE 的 reqwest 版本可以无声息地存在一年。锁文件纪律本身是好的（pnpm-lock.yaml + Cargo.lock 入库、CI `--frozen-lockfile`，ci.yml:37、release.yml:47），缺的只是「锁内容 vs 已知漏洞」的比对。
- **具体建议与权衡**：① `.github/dependabot.yml`（npm、cargo、github-actions 三个 ecosystem，weekly + grouped）零维护成本，先上；② CI 加 `rustsec/audit-check`（读 Cargo.lock 查 RustSec，免装 cargo-audit）与 `pnpm audit --prod`（CI 默认走 npmjs 官方源，端点可用；本机镜像问题不影响 CI）。合计 ~25 行 yaml。权衡：audit-check 对 `main` 分支 push 才有意义（PR 上会重复告警），允许其 failure 不阻塞合并（`continue-on-error`）以免历史债把 CI 染红。
- **置信度**：高（现状为实证；工具配置为常规做法）。

### P2-4 Everything 便携版「下载并提权执行」无完整性校验

- **证据**：src-tauri/src/providers/everything.rs:82-86 从 `https://www.voidtools.com/Everything-{v}.x64.zip` 下载后直接写盘解压（:135-151），全链路无 SHA256 校验；:56-74 抓 voidtools 下载页 HTML 解析最新版本号；:288 一键启用路径调用 download；执行带单次 UAC（README.md:54）。
- **生产后果**：TLS 能挡中间人，但 voidtools 若被仿冒/CDN 供应链出问题，zap 会替攻击者完成「下载 + 解压 + 提权执行」三连——zap 的签名与良好信誉反而成了攻击载荷的搬运工。此外这也是 Defender 启发式的加分项（见 P1-1）。
- **具体建议与权衡**：对已知版本维护一张 SHA256 表（Everything 1.4/1.5 版本少且稳定，每版一行常量），下载后先校验再解压；版本号不在表内（新上游版本）则拒绝或降级为「请用户手动下载」。约 30 行。权衡：上游发新版后 zap 要跟进表——把表做成 map 并留「未知版本 → 静默不自动执行、面板提示手动安装」的兜底即可，不必追版本。
- **置信度**：高（代码路径实证；威胁模型判断为中高）。

### P2-5 卸载残留：vault 数据与自启注册表项在卸载后留存

- **证据**：tauri.conf.json:62-73 bundle 段无 `windows` 配置（无 `deleteAppDataOnUninstall`，Tauri 默认 false）；数据含账号库/剪贴板库（README.md:112：`%APPDATA%\com.ais.zap\zap.db`，密码 DPAPI 加密）；自启为应用运行时写 HKCU Run（lib.rs:1998-2004 经 tauri-plugin-autostart），NSIS 卸载器只清安装目录，不感知该键。
- **生产后果**：两个方向都有客诉可能——隐私敏感用户「卸载了密码怎么还在」；另一类用户卸载后开机偶发「zap 启动失败」提示（Run 键指向已删 exe，Windows 静默忽略但自启管理工具会常年挂着一条死链）。
- **具体建议与权衡**：① 轻方案（推荐）：`deleteAppDataOnUninstall: true` + release-notes 加一句「卸载将同时删除本地数据（含账号库），卸载前请先导出加密备份」——DPAPI 密文绑定本机用户，数据在别的机器上也解不开，删除的实际损失有限；② 或保守方案：保持默认，但在 SECURITY.md/README 数据存储节写明「卸载不删除用户数据，手动删除 %APPDATA%\com.ais.zap」。自启残留用 `installerHooks` 的 `NSIS_HOOK_PREUNINSTALL` 执行 `reg delete HKCU\...\Run /v zap`（与 P0-1 的 hook 文件是同一个，边际成本≈0）。
- **置信度**：中高（deleteAppDataOnUninstall 默认值与卸载器不清 Run 键基于 Tauri 文档默认值与 NSIS 模板行为，未在本机卸载实测，见诚实清单）。

### P2-6 Rust 工具链浮动 + release 无手动补发通道

- **证据**：ci.yml:26-29、release.yml:38-39 均用 `dtolnay/rust-toolchain@stable`（浮动的 latest stable），仓库无 rust-toolchain.toml（src-tauri 目录列表确认）；本机 rustc 1.99.0（2026-09-28）与 CI stable 可能不同版；release.yml:3-5 无 `workflow_dispatch`。CI clippy 是 `-D warnings`（ci.yml:55）——新 stable 引入新 lint 会让 CI 与发版流水线在无代码改动的情况下变红。
- **生产后果**：某天 Rust 新版本发布，tag 推送后 release 构建因新 lint/新编译错误挂掉，且 CI 早已红了没人看——发版被工具链升级阻断。个人开发者最不希望「想发版时流水线坏了」。
- **具体建议与权衡**：加 `src-tauri/rust-toolchain.toml` 钉住 minor 版本（如 `channel = "1.99"`），升级工具链变成显式提交；release.yml 加 `workflow_dispatch:`（需手动填 tag 时的补发入口，配合 P2-1 的幂等上传即可救场）。权衡：钉版本意味着错过 rustc 的修复，对桌面应用完全可接受。
- **置信度**：高。

---

## 3. 发布类功能或流程更改建议（≤5 条）

1. **更新收尾闭环**（对应 P0-1、P1-2）：NSIS `installerHooks` POSTINSTALL 自动拉起新版 + useUpdater 加 24h 周期检查 + Windows 下的 ready 文案修正。三处合计 < 80 行，是本轮收益最大的一组改动。
2. **发版一致性脚本**（对应 P2-2）：`scripts/verify-version.mjs` 断言 package.json / tauri.conf.json / Cargo.toml / Cargo.lock 四处版本相等，接线 pre-push + CI + release.yml（替换现有单文件 jq 校验）；README 测试数去硬编码；CONTRIBUTING.md:47 改为「四处」并列全。
3. **诊断可达性**（对应 P1-3）：panic hook 落 zap.log + 设置页「打开日志文件夹」 + bug_report.yml 增加日志路径引导；顺手把启动期 eprintln 收编进 diag::log。
4. **流水线幂等与留档**（对应 P2-1、P2-6）：release 资产拆 create/upload(--clobber) 两步、upload-artifact 留档、rust-toolchain.toml 钉版本、release.yml 加 workflow_dispatch。
5. **供应链底座**（对应 P2-3、P2-4）：dependabot.yml（npm+cargo+actions，weekly 分组）+ CI 加 audit-check（continue-on-error）+ Everything 下载加 SHA256 校验表。

---

## 4. 诚实清单

- **GitHub 仓库侧设置无法核实**：分支保护是否真的把 CI check 设为合并门禁（CONTRIBUTING.md:21 说「无法合并」，但仓库文件看不到 branch protection）；`TAURI_SIGNING_PRIVATE_KEY` 与密码 secret 是否真的已配置（CONTRIBUTING.md:51 声称已配，本次未验证）。
- **未实际运行 cargo audit / pnpm audit**：本机未装 cargo-audit、pnpm 走 npmmirror 无 audit 端点；当前依赖是否存在已知 CVE 未测得，只审了「无审计机制」这一事实。
- **卸载行为未实测**：`deleteAppDataOnUninstall` 默认 false、NSIS 卸载器不清应用运行时写入的 HKCU Run 键，均基于 Tauri 文档默认值与 NSIS 模板行为的推断，未在本机走一遍安装/卸载验证。
- **installerHooks 重启方案的 API 细节未验证**：`NSIS_HOOK_POSTINSTALL` 里可用的安装模式判断变量（全新安装 vs 更新模式）需真机验证一次。
- **Defender 是否实际拦截本应用不可离线预测**：启发式命中的是概率画像，最终以微软提交分析/真机实测为准；本轮只实证了「特征组合存在且偏高危」。
- **证书渠道价格与准入政策时效未联网核实**（Certum 开源证书现价、SignPath Foundation 当前门槛、Azure Trusted Signing 对个人开发者的开放度均为记忆值，政策多变）。
- **测试计数 117/32 引用自共享简报**，未重跑 cargo test/vitest 复核；仅核实了 README 写死的 113 已落后这一事实。
- **latest.json 端点（github.com）在国内的实际可达率未实测**；「国内更新双源暂不做」为既有拍板，本轮仅记录其含义：更新检查失败静默（useUpdater.ts:57），国内用户可能长期收不到更新提示且无感知——如未来要重开此决策，应基于实测数据。
- **WebView2 缺失场景未验证**：tauri.conf.json 未配置 `webviewInstallMode`，取默认 `downloadBootstrapper`（静默，需联网）；未带 WebView2 且离线的机器（老 LTSC/精简系统）安装会失败，未测。ARM64 现状确认：仅 x86_64 包（release.yml:65/87），无 aarch64 目标；Windows 11 ARM 可经 x64 模拟运行，无原生包、暂无行动建议。
