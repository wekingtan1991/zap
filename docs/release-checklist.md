# 发版前冒烟 checklist（评审 P2-15/D15）

> 自动化门禁（pre-push + CI：cargo test/clippy/fmt、vitest、tsc、check:kb、
> verify-version）全绿只是底线；以下是需要真机手工确认的项，随 release 走。
> 全部来源都是踩过的坑——新增一条的前提是真机踩到且已修复。

## 启动与呼出

- [ ] 冷启动：热键（默认 ctrl+alt+space）呼出面板，焦点在搜索框可直接输入
- [x] zap.log 记有 `startup version=… hotkey=… registered=true clip_enabled=…`——2026-11-06 实读：15:30:53 `startup version=0.18.0 hotkey=ctrl+space registered=true clip_enabled=true`；18:03:21 装包后运行的正式版进程 ProductVersion=0.18.0
- [ ] 睡眠唤醒后热键仍可用、剪贴板监听仍收录（zap.log watcher 无 respawn）

## 拖拽与中转站

- [ ] 资源管理器拖文件→落区条出现→悬停展开→松手→面板开启（开面板开关开/关两态）
- [x] 落区条四角停靠均能呼出（改设置里「落区条位置」为四角各一次，验证细条出现在对应角）——2026-11-06 真机验过，无问题
- [ ] 截图/多文件/网络路径（NAS 死链）拖入不卡不崩
- [ ] 多显示器 + 不同 DPI 缩放下落区条与面板定位正确——⚠️ **本机环境不适用**：实测 `SM_CMONITORS=1`、系统 DPI=96、1536×864 单屏，物理上无法构造多显示器 + 不同缩放，非「测了没过」。用户 2026-11-07 确认按环境豁免处理；换到多屏环境后仍需补验，故保持未勾

## 剪贴板与密码库

- [x] 文本/图片复制→呼出→回贴、仅复制、预览三路——2026-11-06 真机自动化验过前置与回贴：文本写系统剪贴板→`zap.db` 断言 watcher 收录 ✅、`CF_DIB` 图片→`clipboard_images` 59→60 ✅、回贴（Ctrl+V 把标记 A 粘进搜索框→剪贴板改 B→Enter→精确变回 A）✅；2026-11-07 用户真机补完右键菜单「仅复制」「预览大图」两路并确认无问题 → **三路齐，打勾**。测试痕迹已全清（`clipboard_history` 500 / images 59 / 测试标记 0 残留）
- [ ] 浏览器密码管理器复制密码→列表不出现该条（防护）
- [ ] vault 行内明文→30s 自动掩码；Alt+4 快瞄浮层→30s 自动关闭
- [ ] vault 导出→改数据→恢复→账号一致

## 更新链路（发版机）

- [x] 私钥与 tauri.conf.json 公钥同对：发版机本地用 `TAURI_SIGNING_PRIVATE_KEY` 对任意文件跑一次 `pnpm tauri signer sign`，签名成功即私钥可用（2026-11-04 评审 R16：CI 无法校验配对，错配=全绿发版但全体用户更新静默失败）——2026-11-06 三路交叉实证：① `tauri.conf.json` 的 pubkey 字符串与 `~/.tauri/zap.key.pub` 逐字符相同；② 私钥实签成功（空密码，产出 `.sig` 404B）；③ 产物签名的 key id 与配置公钥 key id 一致（`C957897ED1642715`）。注：minisign 签名块载荷 74B = alg(2)+keyid(8)+sig(64)，**不含公钥**，配对只能比 key id
- [ ] 旧版本收到更新提示→下载→自动重启到新版（tauri updater /R 行为）——⚠️ 2026-11-06 18:03 的 0.18.0 是**手动运行 NSIS 安装包**落地的（uninstall.exe mtime 18:03:21、zap.log 无 updater 记录），**不等于本项已验**；仍待 0.17.0 客户端实际触发 tauri updater 验证
- [x] Release 四件套齐（setup.exe / setup.exe.sig / latest.json / setup.sha256）且哈希一致——2026-11-06 v0.18.0 实测：四资产均 HTTP 200（setup.exe 4,406,819B / .sig 432B / latest.json 710B / setup.sha256 65B）；`setup.sha256` 声明值 == 实际下载安装包算得的 SHA256（`383e240f…260e`）；`latest.json` 的 version=0.18.0、url 指向 v0.18.0
- [x] 未签名提示（SmartScreen）文案与 release notes 的 SHA256 对得上——2026-11-06 实测：release notes 正文 SHA256 与 `setup.sha256` 一致，未签名 SmartScreen 提示文案在位

## 系统环境

- [x] 主题切换（暗/浅）全窗口一致（落区条与超级面板恒暗色为既定设计）——2026-11-06 真机验过：主面板浅色态与深色态均已确认，工具 Dock 在浅色下的配色另已单独验过
- [x] 一切操作后 zap.log 无 [E] 级行——2026-11-06 确认：无异常级日志（Everything 提权被 UAC 拒绝时留的 2 条 `[E] 提权启动失败` 属预期行为，非缺陷）
