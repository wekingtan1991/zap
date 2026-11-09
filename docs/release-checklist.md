# 发版前冒烟 checklist（评审 P2-15/D15）

> 自动化门禁（pre-push + CI：cargo test/clippy/fmt、vitest、tsc、check:kb、
> verify-version）全绿只是底线；以下是需要真机手工确认的项，随 release 走。
> 全部来源都是踩过的坑——新增一条的前提是真机踩到且已修复。

## 启动与呼出

- [x] 冷启动：热键（默认 ctrl+alt+space）呼出面板，焦点在搜索框可直接输入——2026-11-07 用户真机验过：退出重开后按热键呼出，不点鼠标直接打字，字符进搜索框，无问题
- [x] zap.log 记有 `startup version=… hotkey=… registered=true clip_enabled=…`——2026-11-06 实读：15:30:53 `startup version=0.18.0 hotkey=ctrl+space registered=true clip_enabled=true`；18:03:21 装包后运行的正式版进程 ProductVersion=0.18.0
- [x] 睡眠唤醒后热键仍可用、剪贴板监听仍收录（zap.log watcher 无 respawn）——2026-11-07 用户真机验过无问题；日志侧 `respawn` 0 命中 / watcher 151 事件

## 拖拽与中转站

- [x] 资源管理器拖文件→落区条出现→悬停展开→松手→面板开启（开面板开关开/关两态）——2026-11-07 用户真机验过两态均无问题
- [x] 落区条四角停靠均能呼出（改设置里「落区条位置」为四角各一次，验证细条出现在对应角）——2026-11-06 真机验过，无问题
- [x] 截图 / 多文件拖入不卡不崩（原含「网络路径 NAS 死链」子项，2026-11-07 用户判定作用不大、不作为发版门槛，故移出）——2026-11-07 用户真机验过截图 / 多文件均无问题。ℹ️ 留档说明：NAS 死链指不可达的 UNC 共享路径，测的是 SMB stat 超时不卡死面板；本版不列为必测项，**非「测了没过」**
- [ ] 多显示器 + 不同 DPI 缩放下落区条与面板定位正确——⚠️ **本机环境不适用**：实测 `SM_CMONITORS=1`、系统 DPI=96、1536×864 单屏，物理上无法构造多显示器 + 不同缩放，非「测了没过」。用户 2026-11-07 确认按环境豁免处理；换到多屏环境后仍需补验，故保持未勾

## 剪贴板与密码库

- [x] 文本/图片复制→呼出→回贴、仅复制、预览三路——2026-11-06 真机自动化验过前置与回贴：文本写系统剪贴板→`zap.db` 断言 watcher 收录 ✅、`CF_DIB` 图片→`clipboard_images` 59→60 ✅、回贴（Ctrl+V 把标记 A 粘进搜索框→剪贴板改 B→Enter→精确变回 A）✅；2026-11-07 用户真机补完右键菜单「仅复制」「预览大图」两路并确认无问题 → **三路齐，打勾**。测试痕迹已全清（`clipboard_history` 500 / images 59 / 测试标记 0 残留）
- [x] 密码防护：桌面密码管理器来源 + 账号库已存密码的复制不落剪贴板历史——**原表述「浏览器密码管理器复制密码→列表不出现该条」经核实写错**（2026-11-07 用户实测该场景会入库）。实现只有两层：① `watcher.rs:138` 的 `is_from_password_manager()` 按剪贴板归属窗口的进程路径匹配 `watcher.rs:27` 的八个桌面 PM 标记；② `watcher.rs:109` 的 `vault::password_cache_hit()` 只与账号库已存密码明文集比对；`watcher.rs:107-110` 注释明写浏览器 PM / 手动复制的**库外密码会漏**，属既有已知缺口、非缺陷。2026-11-07 用户跑 30 秒判定：**复制一条账号库已存密码 → 不入库，防护正常** → 按实现真正保证的范围修正表述并打勾。ℹ️ 浏览器 PM 的库外密码入库是设计行为，不在本项门槛内
- [x] vault 行内明文→30s 自动掩码；Alt+4 快瞄浮层→30s 自动关闭——2026-11-07 用户真机验过无问题；代码侧 `VaultDetailView.tsx:86` 与 `VaultRevealOverlay.tsx:31` 均为 `30_000`
- [x] vault 导出→改数据→恢复→账号一致——2026-11-07 用户真机验过无问题；Rust 单测 `备份读全量与恢复往返` PASS

## 更新链路（发版机）

- [x] 私钥与 tauri.conf.json 公钥同对：发版机本地用 `TAURI_SIGNING_PRIVATE_KEY` 对任意文件跑一次 `pnpm tauri signer sign`，签名成功即私钥可用（2026-11-04 评审 R16：CI 无法校验配对，错配=全绿发版但全体用户更新静默失败）——2026-11-06 三路交叉实证：① `tauri.conf.json` 的 pubkey 字符串与 `~/.tauri/zap.key.pub` 逐字符相同；② 私钥实签成功（空密码，产出 `.sig` 404B）；③ 产物签名的 key id 与配置公钥 key id 一致（`C957897ED1642715`）。注：minisign 签名块载荷 74B = alg(2)+keyid(8)+sig(64)，**不含公钥**，配对只能比 key id
- [x] 旧版本收到更新提示→下载→自动重启到新版（tauri updater /R 行为）——2026-11-07 用户确认**既有真机实证**：设置页「检查更新」可检出新版本并下载后自动安装，呼出面板有更新时 toast 提醒（对应 `useUpdater.ts:52-77` 静默 check + `zap://shown` toast、`UpdateSection.tsx:92/113` 检查/下载按钮）。**本版无需重验的依据**（`6791a57..HEAD` 实读）：`useUpdater.ts` / `UpdateSection.tsx` 一行未动；`tauri.conf.json` 的 updater 段 pubkey 与 endpoints 逐字一致（仅 `version` 0.17.0→0.18.0）；`release.yml` 的 `latest.json` 生成逻辑未动（仅前置发版闸）；后端 `tauri_plugin_updater` 注册未动；签名配对另有三路实证（见上条，key id `C957897ED1642715`）。⚠️ **诚实边界**：本项按「既有实证 + 本次零变更」判定，**未在 0.18.0 产物上重新真机复跑 updater**；11-06 18:03 的 0.18.0 是手动装 NSIS 包，不作为依据。本机现到 `github.com` 主站不通（系统层无代理、直连 `http=000`）属后续网络环境变化，非产品回归
- [x] Release 四件套齐（setup.exe / setup.exe.sig / latest.json / setup.sha256）且哈希一致——2026-11-06 v0.18.0 实测：四资产均 HTTP 200（setup.exe 4,406,819B / .sig 432B / latest.json 710B / setup.sha256 65B）；`setup.sha256` 声明值 == 实际下载安装包算得的 SHA256（`383e240f…260e`）；`latest.json` 的 version=0.18.0、url 指向 v0.18.0。ℹ️ **2026-11-09 起本项脚本化并完成 v0.19.0 补验（全部通过）**：`pnpm release:verify <tag>`（scripts/release/verify-assets.mjs，四道校验：四资产齐 / sha256 对实包 / latest.json 内容 / 签名-公钥 key id 配对）。v0.19.0 结果：`setup.sha256` == 实包 SHA-256 `4ecfc31517e77b52ffd97ea0d3dfa3d5d3a698b1f800ae8635ae46f44fa16eed`、latest.json version/url/signature 一致、key id 配对 `C957897ED1642715`。实测两发现已固化进脚本：①发布 .sig 与 latest.json signature 是「minisign 签名文本整体再 base64」的单行嵌套，解析须先剥一层；②key id 载荷原始字节序与展示 hex 反序，两侧同变换比较即配对。本机 github.com 下载域间歇不通（api.github.com 稳定通）：下载失败自动回落 %TEMP% 上次成功运行的留档，删留档目录可强制重下
- [x] 未签名提示（SmartScreen）文案与 release notes 的 SHA256 对得上——2026-11-06 实测：release notes 正文 SHA256 与 `setup.sha256` 一致，未签名 SmartScreen 提示文案在位

## 系统环境

- [x] 主题切换（暗/浅）全窗口一致（落区条与超级面板恒暗色为既定设计）——2026-11-06 真机验过：主面板浅色态与深色态均已确认，工具 Dock 在浅色下的配色另已单独验过
- [x] 一切操作后 zap.log 无 [E] 级行——2026-11-06 确认：无异常级日志（Everything 提权被 UAC 拒绝时留的 2 条 `[E] 提权启动失败` 属预期行为，非缺陷）

## 持续质量门（scripts/bench，2026-11-07 立项）

> 性能与稳定回归门。数字只与本机基线比（`docs/benchmarks/baseline.json` 记录机器），跨机不可比；换机先 `pnpm bench:baseline` 重建基线。smoke 实例（identifier `com.ais.zap.smoke`）与生产数据物理隔离，构建走 `pnpm build:smoke`。

- [x] L2 性能无回归：发版机上 `pnpm bench && pnpm bench:compare` exit 0——阈值中位 +35%，由 2026-11-07 A/A 实测（同码两跑噪声 ±22%）定为「宁漏报不误报」，真回归 2 倍级照样拦截；篡改验证三路（绿放行 / 3.3 倍拦截 / 1.2 倍放行）已实证。⚠️ **v0.19.0 发版时带注记跳过（2026-11-07 用户拍板）**：三连跑 compare 均拦 9-10 项，但 `git diff 8ab8fee..HEAD -- src-tauri/` 为空（源码零变更）、同项跨跑互差最大 88%、vsc 反比基线快 45-54%、中文名项慢 2-4 倍而英文短项不慢——形态为机器负载噪声（发版机正跑 QoderCN/WorkBuddy/微信等重型应用），非代码回归；本周期与 baseline 同码，L2 绿红均无信息量。**下个含 src-tauri 变更的发版周期，须在空闲机重跑本项**。⚠️ **2026-11-09 v0.20.0 备版已重跑，结果为红、由用户拍板按噪声放行**：`pnpm bench` exit 0，`pnpm bench:compare` exit 1（edge +57% / weixin +58% / yuancheng +70% / vault/get +61%，其余 6 项 0.97–1.05、vsc 0.37）。取证四条：① 同一二进制连跑两轮中位自己翻面 ±68%（abc 384→650ns、vsc 139→234ns、yuancheng 634→388ns），远超本项「±22% 噪声」的校准；② 逐样本分布为 ~1.8× 双峰（edge p25=87 / p50=149 / p75=159ns），中位落在哪簇由调度抽签；③ 11 项 p10 对基线全部 0.93–1.09（vsc 0.38 更快），簇内成本无回归；④ 4 个被拦项的代码路径自基线提交 `9942259` 起零变更（`matching.rs:72`/`:75` 早退分支未被 `6d4a127` 触及，`vault.rs:352` `get` 的 `git diff` 为空）。结论：+35% 阈值落在两簇（间距 ~80%）中间，本机当前状态下判据无判别力。**下个周期整改协议：绑物理核 + 高优先级、判据改 p10/最小值、用同协议重建 `docs/benchmarks/baseline.json`（待实施）**
- [x] L3 冒烟零崩溃：发版机上 `pnpm smoke` 全绿——9 阶段（冷启动×3 / toggle 路径×3+风暴×50 / 搜索 200 键 / 剪贴板翻滚 50 条 / 大图 36MB / kill→重启 / 坏库软着陆 / 日志 [E] 清点），全程 smoke 实例隔离、剪贴板文本自动还原——2026-11-07 本机连续两轮 9/9（145.8s / 157.1s，含坏库软着陆的日志级实证「库不健康，隔离重建」）+ v0.19.0 发版时第三轮 9/9（189.5s，v0.19.0 smoke 实例）。ℹ️ 原生可见性在脚本语境不可断言（呼出即失焦亚秒自动收起），toggle 正确性以 zap.log `shown` 埋点计数为准；e2e（`pnpm bench:e2e`）另行覆盖端到端时延。⚠️ **必须先 `pnpm build:smoke` 再 `pnpm smoke`**：run.mjs 不校验 exe 身份，shared/release/zap.exe 与生产构建同路径共存——2026-11-07 v0.19.0 发版时直接跑 `pnpm smoke`，残留的生产构建 exe 以生产 identifier 运行，S4/S5 测试数据写进生产库（marker 50 条 + 测试图 1 张，已精准删除恢复并留备份）；四败阶段全为「读不到 smoke 日志」的假阴性。ℹ️ **2026-11-09 v0.20.0 备版实测第四轮 9/9（180.0s）**：前置 `pnpm build:smoke` exit 0；S1 就绪 1892/1916/1702ms、S2a 埋点 +2/3（观察项不判定）、S2b +25/50、S3 202 键、S4 9 行 marker、S5 36MB 后存活、S6 旧实例死透重启即绪、S7 日志「库不健康，隔离重建」+ 正常启动、S8 `[E]` 0 条
