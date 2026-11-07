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
- [x] Release 四件套齐（setup.exe / setup.exe.sig / latest.json / setup.sha256）且哈希一致——2026-11-06 v0.18.0 实测：四资产均 HTTP 200（setup.exe 4,406,819B / .sig 432B / latest.json 710B / setup.sha256 65B）；`setup.sha256` 声明值 == 实际下载安装包算得的 SHA256（`383e240f…260e`）；`latest.json` 的 version=0.18.0、url 指向 v0.18.0
- [x] 未签名提示（SmartScreen）文案与 release notes 的 SHA256 对得上——2026-11-06 实测：release notes 正文 SHA256 与 `setup.sha256` 一致，未签名 SmartScreen 提示文案在位

## 系统环境

- [x] 主题切换（暗/浅）全窗口一致（落区条与超级面板恒暗色为既定设计）——2026-11-06 真机验过：主面板浅色态与深色态均已确认，工具 Dock 在浅色下的配色另已单独验过
- [x] 一切操作后 zap.log 无 [E] 级行——2026-11-06 确认：无异常级日志（Everything 提权被 UAC 拒绝时留的 2 条 `[E] 提权启动失败` 属预期行为，非缺陷）

## 持续质量门（scripts/bench，2026-11-07 立项）

> 性能与稳定回归门。数字只与本机基线比（`docs/benchmarks/baseline.json` 记录机器），跨机不可比；换机先 `pnpm bench:baseline` 重建基线。smoke 实例（identifier `com.ais.zap.smoke`）与生产数据物理隔离，构建走 `pnpm build:smoke`。

- [ ] L2 性能无回归：发版机上 `pnpm bench && pnpm bench:compare` exit 0——阈值中位 +35%，由 2026-11-07 A/A 实测（同码两跑噪声 ±22%）定为「宁漏报不误报」，真回归 2 倍级照样拦截；篡改验证三路（绿放行 / 3.3 倍拦截 / 1.2 倍放行）已实证
- [ ] L3 冒烟零崩溃：发版机上 `pnpm smoke` 全绿——9 阶段（冷启动×3 / toggle 路径×3+风暴×50 / 搜索 200 键 / 剪贴板翻滚 50 条 / 大图 36MB / kill→重启 / 坏库软着陆 / 日志 [E] 清点），全程 smoke 实例隔离、剪贴板文本自动还原——2026-11-07 本机连续两轮 9/9（145.8s / 157.1s，含坏库软着陆的日志级实证「库不健康，隔离重建」）。ℹ️ 原生可见性在脚本语境不可断言（呼出即失焦亚秒自动收起），toggle 正确性以 zap.log `shown` 埋点计数为准；e2e（`pnpm bench:e2e`）另行覆盖端到端时延
