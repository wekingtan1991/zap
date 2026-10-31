# 第一轮交叉评审 · 安全与隐私（2026-10-29）

评审人角色：安全与隐私工程师。事实源 = 磁盘当前状态（原「5 文件在途改动」已以 commit ad99093 落库，本报告按该状态审）。全文只读源码，未运行任何修改。

## 1. 角色结论摘要

1. **威胁模型先说清（本报告所有判级的基线）**：这是单用户本地应用，真正的安全边界 = **DPAPI 用户域**（换账户/换机解不开）+ WebView2 进程边界。同 Windows 账户下的任意进程（恶意软件、截图工具、另一个脚本）**天然可读** zap.db 明文剪贴板历史、zap.log、进程内存——这不是漏洞，是 Windows 端共享用户会话的现实（dpapi.rs 模块注释对此口径诚实，README/SECURITY.md 一致）。本报告不把「同用户进程可读」当发现报。
2. **vault 主链路是健康的**：DPAPI CurrentUser 域 + 附加熵（熵非秘密、已声明）；TOTP 密钥与密码同待遇（DPAPI 列 totp_enc）；密码复制走 exclude_from_history + exclude_from_cloud + 45s 阅后即焚；备份文件 PBKDF2-600k + AES-256-GCM 口令加密（backup.rs:13-14,77-105），口令错误统一报错不泄露信息。铁律「密码不进剪贴板历史」在 zap 自身路径上成立——**破口在别处（发现 2）**。
3. **三处真实泄露/暴露路径（P1）**：① zap 托管的 Everything 实例被主动开启**无认证 HTTP 服务**（127.0.0.1），全盘文件索引对本地任意进程与 DNS rebinding 页面开放；② 剪贴板入库的密码管理器过滤只认 8 个桌面 PM 进程名，**浏览器内置密码管理器**（Chrome/Edge 是最多人用的密码管理器）复制的密码会全文入历史库并在搜索列表回显；③ 保护级别不一致——备注/用户名/超级面板抓取文本走普通剪贴板写入（进 Win+V 历史与云同步），只有密码/TOTP 走受保护通道。
4. **更新与日志链路是干净的**：latest.json 信任链 = TLS + GitHub + 固化 minisign 公钥（tauri.conf.json:76）+ 发布物签名（release.yml:54-76），降级被插件版本比较挡住；zap.log / capture.log 内容核查无密码、无剪贴板内容、无 URL token（详见发现 8 的正面结论与诚实清单）。CSP 无 unsafe-inline 脚本、connect-src 不含任意外域——webview 被注入后的外呼与 exfil 面收得很窄。
5. **「无遥测」基本成立**：仅有的外联是 voidtools 下载页/下载（用户显式启用 Everything）、127.0.0.1 探测、启动时 updater 静默 check 一次 GitHub Releases（useUpdater.ts:50，会向 GitHub 暴露 IP 与请求，属更新检查惯例而非遥测，无用户数据上报）。与拍板一致，无需推翻。

## 2. 发现清单（按 P0 > P1 > P2；本次无 P0）

### P1-1 zap 主动把 Everything 配成「本机无认证全盘文件索引 HTTP 服务」

- **证据**：`src-tauri/src/providers/everything.rs:162-166`（FIXED 三行：`allow_http_server=1`、`http_server_enabled=1`、`http_server_bindings=127.0.0.1`，注释明说必须开 HTTP 才能被 zap 查询）；端口 40000-59999 伪随机（everything.rs:232-238）；查询端零认证零 token：`src-tauri/src/providers/files.rs:42,89-92`（裸 GET `http://127.0.0.1:{port}/?json=1&...`）。
- **威胁场景**：谁·怎么利用——① 同机任意进程（浏览器沙箱里的 JS 除外）可直接 `GET http://127.0.0.1:<port>/?json=1&s=<词>` 枚举用户全盘文件名（含 U 盘、NAS 映射盘里 Everything 索到的内容），无需任何凭证；② DNS rebinding：用户浏览恶意网页，页内脚本把攻击域名 rebind 到 127.0.0.1 并猜/扫端口（窗口 2 万个，rebinding 扫描是成熟手法），命中后**同源读取**响应——整个磁盘的文件名清单可被远程页面拖走。Chrome 的 PNA（私网访问保护）收窄了这条，Firefox/部分内嵌 WebView 不设防。暴露的不是文件内容，是「你机器上有什么」的全量地图——社工与定向攻击的高价值情报。
- **建议与权衡**：最小修 = seed_ini 时生成随机用户名/口令写入 `http_server_username=` / `http_server_password=`（Everything 原生支持），files.rs 请求带 Basic Auth（端口+凭证内存持有，与现有 ini 读取同路径，files.rs:53-67 已在读 ini）。更彻底 = 改用 Everything SDK DLL（IPC，无 HTTP 面），但改动大，不作为一轮内要求。权衡：加认证不影响 zap 自身查询体验，零便捷损失。
- **置信度**：高（配置与查询链路均为代码事实；rebinding 的实际可行性依浏览器而变，已如实标注）。

### P1-2 密码管理器来源过滤漏掉浏览器内置密码管理器（最常见来源），密码全文入历史库并回显

- **证据**：`src-tauri/src/providers/clipboard.rs:191-200`（PASSWORD_MARKERS 仅 keepass/bitwarden/1password/enpass/dashlane/lastpass/roboform/keyguard 8 个桌面进程名）；`clipboard.rs:276-291`（非命中文本全文 INSERT 进 clipboard_history）；`clipboard.rs:460-467`（标题=首行前 60 字符，**回显在搜索结果里**）；`lib.rs:1571-1594`（快速粘贴热键可把历史内容再贴出）。vault.rs:2 的铁律「密码永不出现在搜索结果、日志或剪贴板历史中」在此路径失守。
- **威胁场景**：用户在 Chrome/Edge 的 `chrome://password-manager`（或任何未列入清单的 PM：Proton Pass、KeeWeb、Firefox Lockwise 等）点「复制密码」——剪贴板 owner 是 chrome.exe/msedge.exe，不在 markers 里 → 密码明文进 zap.db，之后每次呼出面板都可能出现在剪贴板历史列表、参与搜索打分、被快速粘贴热键注入到当前窗口。滚动窗口 500 条（clipboard.rs:188），明文驻留周期长。
- **建议与权衡**（均不牺牲便捷）：**首选**：入库前把文本与账号库密码集比对，命中即跳过入库（且顺带清掉历史中已有的同值行）。vault 规模个人级（<200 条），DPAPI 解密耗时亚毫秒；解密结果缓存于内存、在 vault_add/update/import/backup_restore 后失效——代价是明文密码集常驻进程内存（与「详情页明文已过 IPC」的现状同量级，可接受；不放心可只在「owner 是浏览器进程」时才现场比对）。次选：扩充 markers 加浏览器进程 + 仅当文本高熵（含大小写数字符号且 ≥12 位）时豁免浏览器来源——误判面大，不如首选。最低限度：README「隐私边界」补一句如实声明。
- **置信度**：高（过滤逻辑与入库路径为确定代码行为；「Chrome 复制密码时 owner 是 chrome.exe」基于 Windows 剪贴板语义推断，未真机验证——列入诚实清单）。

### P1-3 保护通道不一致：备注/用户名/超级面板文本可进 Win+V 历史与云剪贴板

- **证据**：密码与 TOTP 走 `copy_text_private`（exclude_from_history + exclude_from_cloud）：vault.rs:59-71,295,368。而 **备注** 复制走普通 `copy_text`：vault.rs:322——且 vault.rs:315 注释自己写着「备注可能含敏感信息」；**用户名** 同走普通通道：vault.rs:281；**超级面板**「复制内容」走普通 set_text：superpanel.rs:254-260。begin_self_copy 只压 zap 自身入库，不排 Win+V/云（clipboard.rs:23-25）。
- **威胁场景**：Windows 云剪贴板「跨设备同步」开启时（设置→系统→剪贴板），用户复制的备注（典型内容：恢复码、辅助密钥、密码提示）会上传到用户 Microsoft 账户的云剪贴板；Win+V 历史明文留存（本机，按威胁模型可接受，但密码通道特意排除了它，说明产品意图是「vault 相关内容不进 Win+V」——备注/用户名违反了同一意图）。超级面板抓取的选中文本（可能是屏幕上正在查看的密码）同理。
- **建议与权衡**：notes / username / superpanel copy 三处改走 `copy_text_private`（现成函数，一行级改动，零便捷损失）。用户名改不改可以争论（用户名常需在网页粘贴，exclude_from_history 不影响粘贴）——但备注与超级面板没有理由不改。
- **置信度**：高。

### P2-4 open_item 的 settings 分支把前端可控 target 拼进 `cmd /C start`，cmd 元字符未中和

- **证据**：`src-tauri/src/lib.rs:1150-1155`（`spawn_hidden(Command::new("cmd").args(["/C","start","", &item.target]))`）；target 来自前端回传的 SearchItem（lib.rs:1108），而 SearchItem 全字段由 webview 提供（mod.rs:33-51 注释也写明 target 语义宽松）。Rust 的参数拼装会原样下发未含空格/引号的参数——`target = "ms-settings:&calc"` 这类值经 cmd 解析时 `&` 是命令分隔符。
- **威胁场景**：前提是 webview 先被 XSS（CSP `script-src 'self'`（tauri.conf.json:58）已把内联注入挡住，故这是纵深防御缺口而非直接可利用漏洞）。一旦成立，攻击者可借 settings 分支把任意命令拉起（比 `_ => open_url` 兜底更直接）。另注：lib.rs:1133-1134 注释称「这是资源管理器"运行"同样的解析路径」不准确——Run 对话框走 ShellExecute，不经过 cmd 的 `&` 解析。
- **建议与权衡**：settings 类目的 target 全部来自静态枚举（settings.rs:26-80 全为 `ms-settings:` URI 与经典工具名）——加一个白名单校验（`starts_with("ms-settings:")` 或命中已知 .cpl/.msc/工具名集合）即可，成本低；或改 ShellExecuteW 直启摆脱 cmd。顺手修正注释。
- **置信度**：中（注入机制确定；可达性完全依赖「webview 先被攻破」这一未发生的前提）。

### P2-5 Everything 下载安装链无完整性校验即提权运行

- **证据**：`everything.rs:72-98`（HTTPS 下载 zip，仅看 HTTP 状态，无哈希/签名校验）；`everything.rs:134-153`（解压）；`everything.rs:244-268`（写批处理 + `-Verb RunAs` 提权启动）；`everything.rs:100-103`（落盘 %LOCALAPPDATA%\zap\everything）。
- **威胁场景**：企业代理/被装了根证书的机器/恶意 Wi-Fi 上，MITM 用受信 CA 签发 voidtools.com 的伪造证书 → 投毒 zip → 用户点 UAC「是」→ 提权执行任意代码。HTTPS 保证了普通网络下的安全，但「下载即提权运行」的组合值得一道签名验证兜底。
- **建议与权衡**：解压后、提权前对 Everything.exe 做 Authenticode 校验（WinVerifyTrust，验证签发者为 voidtools——Everything 官方分发是签名的）。一次 WinAPI 调用，成本低。若 voidtools 实际未签名，退而校验已知版本的 SHA-256（内置表已有版本号，everything.rs:18-24）。
- **置信度**：高（缺校验是事实；「Everything.exe 带 voidtools 签名」需真机确认——诚实清单）。

### P2-6 release 工作流：第三方 action 未固定 SHA，且签名私钥同 runner 暴露

- **证据**：`.github/workflows/release.yml:16,30,33,39,42`（`actions/checkout@v4`、`pnpm/action-setup@v4`、`setup-node@v4`、`dtolnay/rust-toolchain@stable`、`Swatinem/rust-cache@v2` 全是分支/主版本 tag 引用）；`release.yml:54-58`（TAURI_SIGNING_PRIVATE_KEY 注入同一次 build 的环境）。
- **威胁场景**：任一 action 的 tag 被劫持（历史真实案例：tj-actions/changed-files）→ 恶意代码在持有签名私钥的 runner 上运行 → 私钥外泄 → 攻击者可签发恶意更新，latest.json 信任链整个失效。tag 推送即出包（release.yml:3-5），无人工门。
- **建议与权衡**：五个 action 全部固定到 commit SHA（一次性工作，以后升级时顺手换）；可选：给 release 加 environment 保护（手动批准）。NSIS 安装包无 Authenticode 签名（workflow 无证书步骤）属分发信任缺口——用户手动下载 setup.exe 会见 SmartScreen 警告；updater 路径有 minisign 保护不受影响。代码签名证书有年费，属成本决策，不强推，但应在 README 说明。
- **置信度**：高（事实）；利用概率低、加固代价低，故 P2。

### P2-7 卸载残留：明文剪贴板历史与日志在卸载后留存

- **证据**：数据全部落在 `%APPDATA%\com.ais.zap\`（lib.rs:1781-1784：zap.db + zap.log；hotkey_capture.rs:20-23：capture.log；icons\），另一处 `%LOCALAPPDATA%\zap\everything`（everything.rs:100-103）。安装包仅 NSIS（tauri.conf.json:64），Tauri 默认 NSIS 卸载器不清理 AppData。
- **威胁场景**：共享电脑/设备转手/他人接手登录同一 Windows 账户场景，卸载 zap 后 zap.db 里最多 500 条明文剪贴板历史（含发现 2 漏进来的密码）、capture.log、icons 缓存、以及 DPAPI 密文 vault 仍在磁盘上可读（同账户下可读——发现 1 的 Everything HTTP 若还开着，还多一条程序化读取通道）。
- **建议与权衡**：NSIS 自定义卸载钩子删除两个目录（Tauri 支持 installer/installer-hooks；可在卸载页给「同时删除历史数据」勾选，默认勾）；至少在 README「数据位置」段落（README.md:112 已有雏形）补一句「卸载不会删除这些文件，请手动清理」。不做也属行业常见现状（大量应用如此），故 P2。
- **置信度**：中高（「Tauri NSIS 默认不清 AppData」依据其文档与惯例，未跑卸载实测——诚实清单）。

### P2-8 快速粘贴热键可把焚除窗口内的账号密码注入前台窗口

- **证据**：`lib.rs:1571-1594`（quick_paste_latest 优先取**当前剪贴板文本**直接注入）；`vault.rs:294-298`（密码复制后明文留在剪贴板直至焚除，默认 45s，appconfig.rs:11）。
- **威胁场景**：用户在 zap 复制密码（焚除倒计时中）→ 切到聊天窗口本想粘贴别的东西 → 习惯性按下快速粘贴热键 → **密码贴进聊天/共享屏幕/错误表单**。另一变体：焚除未启用（pwd_clear_secs=0）时窗口无限长。这是 Ctrl+V 注入面里唯一涉及受保护内容的误注入路径；paste_to_prev 的目标窗口焦点校验（winsys.rs:156-167）挡得住「贴错窗口句柄」，挡不住「内容本身不该贴」。
- **建议与权衡**：焚除窗口期内（序列号 == 受保护复制时的 seq 且未到期）快速粘贴跳过注入或退化为「回源历史库」——vault 复制不进历史库，届时 latest_clip 返回上一条普通内容，语义自然回到用户预期；基础设施（clipboard_seq、schedule_clear 的 seq 判定）都在，改动小。零新增交互、零便捷损失（45s 后恢复原语义）。
- **置信度**：中高（代码路径确定；触发依赖用户操作顺序，属真实但非高频）。

### P2-9 capabilities 非最小权限：opener 挂满三个窗口，另有疑似未用的插件权限

- **证据**：`src-tauri/capabilities/default.json:5-8`（`windows: ["main","superpanel","stagedrop"]` + `opener:default`）；`desktop.json:8-16`（global-shortcut:default、autostart:default 仅挂 main）。前端 grep 全量核对：JS 侧只直接使用 plugin-dialog / plugin-updater / plugin-process（均在 main 窗口，与 desktop.json 一致）；`@tauri-apps/plugin-opener`、global-shortcut、autostart 在前端**零引用**（opener/global-shortcut/autostart 全部走 Rust 侧调用，Rust 侧调用不需要 capability 授权）。
- **威胁场景**：多余的 capability = webview 被攻破后多出来的可用 IPC 面。当前三者窗口都是第一方内容且 CSP 收紧，实际风险低——这是「最小权限纪律」问题不是漏洞。
- **建议与权衡**：default.json 从 windows 列表去掉 stagedrop/superpanel（或至少去掉 opener:default）；desktop.json 验证后裁剪 global-shortcut/autostart。改动即时生效、无行为影响（前提：确认无隐藏的 JS 调用——见诚实清单）。
- **置信度**：中（「前端未用」由全量 grep 得出；不能排除插件运行时对 capability 的隐式要求，裁剪前跑一遍功能回归即可证伪）。

### P2-10 智能导入邀请用户搬运凭据文本，其「来源复制」已在剪贴板历史中（P1-2 的同根放大器）

- **证据**：导入文案明示场景「粘贴记事本 / **聊天记录**里的账号文本」（VaultImportView.tsx:80）；那段文本在用户于记事本/QQ 里复制它时已被 zap watcher 入库（clipboard.rs:276-291，来源进程不在 markers），导入完成后无人清理。
- **威胁场景**：用户按产品引导完成一次智能导入 = 主动把一批含密码的文本留在了 500 条滚动窗口的剪贴板历史里，与「vault 铁律」的目标直接相悖；快速粘贴/搜索都可能再把它带出来。
- **建议与权衡**：vault_import 成功后，后端按导入原文（命令参数本就经过后端）做一次 `DELETE FROM clipboard_history WHERE content = ?` 的等值清理（注意 content 有 UNIQUE 约束，等值匹配无歧义）；再在导入完成文案里提一句「来源文本已从剪贴板历史移除」。零便捷损失，闭环了产品自己制造的一类暴露。
- **置信度**：高（行为链路全部可从代码推出）。

## 3. 安全类功能或流程更改建议（≤5 条）

1. **给托管 Everything 上认证**（P1-1）：seed_ini 生成随机凭证 + files.rs 请求带 Basic Auth。若本轮只想做一件事，做这件——它是 zap 主动降低用户本机安全姿态的唯一一处。
2. **统一「受保护复制」通道**（P1-3 + P2-8 + P2-10 打包）：notes/username/superpanel 改 copy_text_private；焚除窗口内快速粘贴跳过；vault_import 成功后等值清理历史。三者共享同一组现成原语（copy_text_private / clipboard_seq / begin_self_copy），一个 PR 量级。
3. **入库密码比对护栏**（P1-2 首选方案）：clipboard watcher 入库前与 vault 密码集比对（解密缓存随 vault 写失效），命中跳过并清历史残留；同时把「浏览器密码管理器不设防」写进 README 隐私边界说明。
4. **供应链最小加固**（P2-5 + P2-6）：release.yml 五个 action 固定 commit SHA；Everything.exe 提权前做 Authenticode/哈希校验。
5. **卫生三项**（P2-4 + P2-7 + P2-9）：open_item settings 分支白名单校验；NSIS 卸载钩子清 AppData（或 README 明示）；capabilities 裁剪 + 跑一遍功能回归。

## 4. 诚实清单（未验证 / 无法在只读评审下确定）

- **Chrome/Edge 复制密码时剪贴板 owner 是否确为浏览器进程**：P1-2 的触发前提基于 Windows 剪贴板语义推断，未真机复现（部分浏览器扩展走中间进程时会进一步改变 owner）。
- **DNS rebinding 当下的实际可行性**：Chrome 的 Private Network Access 强制进度、Firefox 的最新状态我未联网核实版本号，P1-1 的威胁场景措辞已按「浏览器依版本而变」降格为条件成立。
- **Everything.exe 是否带 voidtools Authenticode 签名**：影响 P2-5 建议的落地形式（签名校验 vs 哈希表）。
- **tauri-plugin-updater 的降级拒绝**：依据插件文档「新版本号不大于当前则不提供更新」，未实测；若确认依赖此行为，latest.json 生成端（release.yml:70-76）无额外防线，GitHub 仓库被攻破时可发起降级攻击（仍需过签名）。
- **Tauri NSIS 卸载是否真的不清理 AppData**：依据 Tauri 惯例与模板推断，未跑安装/卸载实测。
- **capabilities 裁剪是否会破坏隐式依赖**：P2-9 的「疑似未用」来自前端源码 grep，未做运行时验证。
- **`is_from_password_manager` 对提权进程/高完整性进程 owner 的表现**：PROCESS_QUERY_LIMITED_INFORMATION 通常可查，未验证极限场景。
- **内存明文清理时机（领域问题的正面回答）**：30s 自动掩码是**视觉层**的——掩码态 DOM 只渲染 `••••••••`（VaultDetailView.tsx:188-192），明文不在 DOM；但明文自 vault_get 起整个详情页生命周期都在 JS 内存（React state），编辑表单与 reveal 浮层同理；Rust 侧 String drop 不抹零（焚除线程持有的明文、DPAPI 解密缓冲除外——后者已 LocalFree，dpapi.rs:49-51,84-86）。这是「便捷优先」拍板下的已知取舍，零化（zeroize）只能覆盖部分副本、收益边际，不建议为此引入；仅在此留档。
- **密码生成器模偏**：`n % 62`（password.ts:8）理论偏差约 4/2^32 每字符，密码学上可忽略，不列为发现。
- **未逐行审读**：apps.rs（36KB）、stage.rs/stagedrop.rs 的拖拽与剪贴板格式细节、jsontools/texttools 全文、ci.yml——按威胁面优先级抽查了入口与外联点，未做全量行审。
