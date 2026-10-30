# 发版前冒烟 checklist（评审 P2-15/D15）

> 自动化门禁（pre-push + CI：cargo test/clippy/fmt、vitest、tsc、check:kb、
> verify-version）全绿只是底线；以下是需要真机手工确认的项，随 release 走。
> 全部来源都是踩过的坑——新增一条的前提是真机踩到且已修复。

## 启动与呼出

- [ ] 冷启动：热键（默认 ctrl+alt+space）呼出面板，焦点在搜索框可直接输入
- [ ] zap.log 馧行 `startup version=… hotkey=… registered=true clip_enabled=…`
- [ ] 馐面板四角停靠均能呼出（改设置后逐角验证）
- [ ] 睡眠唤醒后热键仍可用、剪贴板监听仍收录（zap.log watcher 无 respawn）

## 拖拽与中转站

- [ ] 资源管理器拖文件→落区条出现→悬停展开→松手→面板开启（开面板开关开/关两态）
- [ ] 萱图/多文件/网络路径（NAS 死链）拖入不卡不崩
- [ ] 多显示器 + 不同 DPI 缩放下落区条与面板定位正确

## 剪贴板与密码库

- [ ] 文本/图片复制→呼出→回贴、仅复制、预览三路
- [ ] 浏览器密码管理器复制密码→列表不出现该条（防护）
- [ ] vault 行内明文→30s 自动掩码；Alt+4 快瞄浮层→30s 自动关闭
- [ ] vault 导出→改数据→恢复→账号一致

## 更新链路（发版机）

- [ ] 旧版本收到更新提示→下载→自动重启到新版（tauri updater /R 行为）
- [ ] Release 三件套齐（setup.exe / latest.json / .sig / setup.sha256）且哈希一致
- [ ] 未签名提示（SmartScreen）文案与 release notes 的 SHA256 对得上

## 系统环境

- [ ] 主题切换（暗/浅）全窗口一致（落区条与超级面板恒暗色为既定设计）
- [ ] 一切操作后 zap.log 无 [E] 级行
