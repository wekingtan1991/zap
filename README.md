# zap ⚡

Windows 桌面快捷启动器：一个呼之即来、挥之即去的小面板，把应用、文件、窗口、剪贴板、账号装进同一个输入框。

Tauri 2 + React 19 + Rust，仅支持 Windows 10/11。

> **English**: zap is a Windows desktop launcher that puts apps, files, windows,
> clipboard history and a password vault into one keystroke-away panel.
> Press `Ctrl+Alt+Space` (configurable) to summon it. Built with Tauri 2 + React + Rust.
> Data stays local on your machine (see [数据存储](#数据存储)) — nothing is uploaded anywhere.

## 安装说明（SmartScreen 提示）

从 Releases 下载的安装包**未做代码签名**，Windows 首次运行会弹出：

> Windows 已保护你的电脑

点击「更多信息 → 仍要运行」即可完成安装。这是未签名安装包的正常提示，不是病毒警告。
介意者可从本仓库源码自行构建（见 [开发](#开发)），构建产物完全一致。

## 功能

### 统一搜索

输入即搜，各来源汇入同一份结果（按相关度排序，带类型角标或真实图标）：

| 来源 | 说明 |
| --- | --- |
| 应用 | 开始菜单 `.lnk` 索引，提取目标程序真实图标；「卸载 XX」类快捷方式自动过滤，面板每次呼出后台重建索引，新装/卸载无需重启 |
| 全盘文件 | 走 [Everything](https://www.voidtools.com/) 的 HTTP/JSON 服务；未安装时可在面板里一键启用（自动下载官方便携版约 2MB → 预置配置 → 单次 UAC 提权启动），未就绪时静默休眠、不影响其他来源 |
| 窗口 | 枚举当前打开的窗口，选中即切换到前台（最小化的自动还原） |
| 剪贴板 | 本地历史（SQLite），选中回填剪贴板并去重置顶 |
| 最近 | 系统「最近使用」里的文件 |
| 设置 | 40+ 个 Windows 设置页直达（`ms-settings:`），支持中文/英文/缩写 |
| 网页 | 网址直达；其余兜底为百度 / 必应 / 谷歌搜索 |

匹配支持拼音：中文应用名可用全拼（`weixin`）或首字母（`wx`）命中。

### 账号库

输入 `pw` 进入保护域，本地账号密码管理：

- 密码经 **Windows DPAPI** 加密入库，与剪贴板历史隔离
- 快捷键：`Enter` 复制用户名 · `Alt+1` 打开网址 · `Alt+2` 复制密码（阅后即焚）· `Alt+3` 删除 · `Alt+4` 查看密码 · `Alt+5` 置顶/取消 · `Alt+6` 编辑
- **任何**账号库复制（含用户名/网址）都不会进入剪贴板历史；复制后面板内显示成功反馈
- 常用账号可置顶（排最前带 📌），复制操作会刷新「最近使用」让常用条目自然前移
- 表单内置**密码生成器**（16 位强密码）；编辑时密码留空 = 保持不变
- 搜索覆盖标题/用户名/网址/备注
- 支持智能导入：把记事本/聊天记录里的账号文本整段粘贴，自动解析字段后逐条校对入库

### 设置

输入栏末尾的齿轮图标、`Ctrl+,` 或托盘菜单「设置…」均可打开，改动即时生效：

- **呼出热键**：点击输入框录制新组合，保存时实时试注册，被占用会给出原因（启动时存档失效也会自动回退默认键）
- **剪贴板历史**：总开关 + 保留条数；托盘有同名快捷开关，两处状态同步
- **密码剪贴板自动清空**：复制账号库密码后延时自动清除，期间你复制了别的内容则不覆盖
- **开机自启**：与托盘为同一开关

### 常驻与呼出

- 托盘图标：左键呼出面板；右键菜单含「设置…」、剪贴板历史与开机自启开关、退出
- 面板失焦自动隐藏（弹出 UAC 等系统对话框时保持可见）
- 已有实例运行时再次启动 = 呼出面板

## 快捷键

| 按键 | 作用 |
| --- | --- |
| `Ctrl+Alt+Space` | 呼出 / 隐藏面板 |
| `↑` `↓` / `Enter` | 选择 / 打开 |
| `Esc` | 关闭面板（表单/导入页返回上级） |
| `pw` | 进入账号库 |
| `Ctrl+,` | 打开设置 |
| `Ctrl+Enter` | 保存新增账号表单 |

## 数据存储

- `%APPDATA%\com.ais.zap\` — `zap.db`（剪贴板历史 + 账号库，账号密码经 Windows DPAPI 加密）、`icons\`（应用图标缓存，键为路径+mtime，应用更新自动失效）
- `%LOCALAPPDATA%\zap\everything\` — 托管的 Everything 便携版

**隐私**：以上数据全部只存在本机，zap 不联网、不上传任何数据（Everything 仅在本地回环查询）。上传到 GitHub 的只是源码，不含任何个人数据。

## 开发

前置要求：Node.js + pnpm、Rust（MSVC 工具链）。

```bash
pnpm install          # 安装前端依赖
pnpm tauri dev        # 开发运行
pnpm tauri build      # 打包 NSIS 安装包
```

测试（在 `src-tauri/` 下）：

```bash
cargo test --lib                                    # 单元测试
cargo test --lib -- --ignored --nocapture           # 真实开始菜单图标提取冒烟（机器相关）
```

## 架构速览

- `src-tauri/src/providers/` — 每个搜索来源一个模块（apps / clipboard / everything / files / recent / settings / sysutil / vault / web / winsys），统一产出 `SearchItem`
- `src-tauri/src/providers/mod.rs` — `SearchEngine` 汇总各源，`score_match` 统一打分（直连 → 拼音全拼 → 首字母）
- `src-tauri/src/lib.rs` — 窗口/托盘/全局热键/失焦隐藏、Everything 托管安装流程、命令入口
- `src/App.tsx` — 单面板 React 组件，输入防抖调用后端 `search`，账号库保护域与表单/导入页状态机
