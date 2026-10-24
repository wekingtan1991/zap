# zap ⚡

Windows 桌面快捷启动器：一个呼之即来、挥之即去的小面板，把应用、文件、窗口、剪贴板、账号装进同一个输入框。

Tauri 2 + React 19 + shadcn/ui + Rust，仅支持 Windows 10/11。

> **English**: zap is a Windows desktop launcher that puts apps, files, windows,
> clipboard history and a password vault into one keystroke-away panel.
> Press `Ctrl+Alt+Space` (configurable) to summon it. Built with Tauri 2 + React + Rust.
> Data stays local on your machine (see [数据存储](#数据存储)) — nothing is uploaded anywhere.

## 功能

### 统一搜索

输入即搜，各来源汇入同一份结果（按相关度排序，带类型角标或真实图标）：

| 来源 | 说明 |
| --- | --- |
| 应用 | 开始菜单 `.lnk` 索引，提取目标程序真实图标；「卸载 XX」类快捷方式自动过滤，面板每次呼出后台重建索引，新装/卸载无需重启；启动次数参与排序，越用越靠前 |
| 全盘文件 | 走 [Everything](https://www.voidtools.com/) 的 HTTP/JSON 服务；zap 启动时自动检测，未运行且已安装则自动拉起，未安装时可在面板里一键启用（自动下载官方便携版约 2MB → 预置配置 → 单次 UAC 提权启动），未就绪时静默休眠、不影响其他来源 |
| 窗口 | 枚举当前打开的窗口，选中即切换到前台（最小化的自动还原） |
| 剪贴板 | 本地历史（SQLite），选中回填剪贴板并去重置顶；常用条目可固定（📌 置顶展示，不参与保留条数淘汰）；焦点自动交还呼出前的窗口并模拟 Ctrl+V 粘贴，一步到位（粘贴设置里可关） |
| 最近 | 系统「最近使用」里的文件 |
| 设置 | 40+ 个 Windows 设置页直达（`ms-settings:`），支持中文/英文/缩写 |
| 网页 | 网址直达；其余兜底为百度 / 必应 / 谷歌搜索 |

匹配支持拼音：中文应用名可用全拼（`weixin`）或首字母（`wx`）命中。

### 账号库

输入 `pw` 进入保护域，本地账号密码管理。用法接近记事本：**列表只看标题 → 点开看详情 → 想用哪行点哪行的复制**。

- 列表只显示标题（保存时会提示标题用于记录这是什么账号，如「行云账号」）
- 点击标题打开**详情页**，逐行展示 用户名 / 密码 / 网址 / 备注
- 密码**默认掩码**（`••••••••`），可切换显示；**每行末尾一枚一键复制图标**，复制后图标变为 ✓ 并弹出反馈
- 密码经 **Windows DPAPI** 加密入库，与剪贴板历史隔离；复制密码走阅后即焚（默认 45 秒自动清除，期间复制别的内容则不覆盖）
- **任何**账号库复制（含用户名/网址/备注）都不会进入剪贴板历史
- 详情页内可直接 **编辑 / 删除（二次确认）/ 置顶**；列表每行右侧也有置顶/取消置顶按钮
- 常用账号置顶后排最前（标题带 📌），复制操作会刷新「最近使用」让常用条目自然前移
- 表单内置**密码生成器**（16 位强密码）；编辑时密码留空 = 保持不变
- 搜索覆盖标题/用户名/网址/备注；空关键词时列出全部账号
- 支持**智能导入**：把记事本/聊天记录里的账号文本整段粘贴，自动解析字段后逐条校对入库；标题缺省的条目按 用户名 → 网址 → 「未命名账号」自动命名，整批一个事务，不会出现"导入一半失败还没提示"
- **加密备份**：设置 → 账号库 → 导出/导入。导出为口令加密文件（PBKDF2 600k 轮派生密钥 + AES-256-GCM，口令自定），全字段还原（含置顶），换机重装有出路

### Everything 自动检测与启动

- zap 启动时自动检测 Everything 是否在运行：**未运行且已安装 → 自动拉起**（会弹一次 UAC，请点"是"）
- **未安装则不自动下载**，面板列表保留手动入口由你决定
- 拉起失败（含取消 UAC）后 5 分钟内不重复打扰；可在 设置 → 「自动启动 Everything」 关闭

### 设置

输入栏末尾的齿轮图标、`Ctrl+,` 或托盘菜单「设置…」均可打开，改动即时生效：

- **呼出热键**：点击输入框录制新组合，保存时实时试注册，被占用会给出原因（启动时存档失效也会自动回退默认键）
- **剪贴板历史**：总开关 + 保留条数 + 选中后自动粘贴；托盘有历史开关同名项，两处状态同步
- **密码剪贴板自动清空**：复制账号库密码后延时自动清除
- **开机自启**：与托盘为同一开关
- **自动启动 Everything**：见上一节
- **检查更新**：启动时静默检查新版本（有新版会在下次呼出面板时提示），设置页可手动检查并一键下载安装，重启即完成更新

### 自动更新

- 更新包经 **minisign 签名**校验后才安装（公钥内置，第三方无法伪造更新）
- 更新源为 GitHub Releases（`releases/latest/download/latest.json`），离线或网络失败时静默跳过、不打扰
- 设置页「检查更新」→「下载并安装」→「重启以完成更新」，全程无需重新下载安装器

### 常驻与呼出

- 托盘图标：左键呼出面板；右键菜单含「设置…」、剪贴板历史与开机自启开关、退出
- 面板失焦自动隐藏（弹出 UAC 等系统对话框时保持可见）；**收起时即清空搜索词**，每次呼出输入框都是空的
- 同一会话内（列表 ↔ 详情页之间）保留搜索词
- 已有实例运行时再次启动 = 呼出面板

## 快捷键

| 按键 | 作用 |
| --- | --- |
| `Ctrl+Alt+Space` | 呼出 / 隐藏面板 |
| `↑` `↓` / `Enter` | 选择 / 打开（账号库中为打开详情页） |
| `Ctrl+Enter` | 应用/文件条目：打开所在文件夹并定位 |
| `Ctrl+Shift+Enter` | 应用条目：以管理员身份运行 |
| `Ctrl+Shift+C` | 应用/文件条目：复制完整路径 |
| `Esc` | 关闭面板（表单/导入/详情页返回上级） |
| `pw` | 进入账号库 |
| `Ctrl+,` | 打开设置 |
| `Ctrl+Enter` | 保存账号表单 |
| `Alt+1` ~ `Alt+6` | 账号库列表内：网址 / 复制密码 / 删除 / 查看密码 / 置顶 / 编辑 |

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
pnpm build            # 仅前端类型检查 + 构建（tsc && vite build）
```

测试与规范（在 `src-tauri/` 下）：

```bash
cargo test --lib                                    # 单元测试（49 个）
cargo test --lib -- --ignored --nocapture           # 真实开始菜单图标提取冒烟（机器相关）
cargo fmt --check                                   # 格式检查（CI 同款）
cargo clippy --all-targets -- -D warnings           # lint（CI 同款，零警告）
```

前端用 Tailwind CSS v4 + shadcn/ui（组件在 `src/components/ui/`，主题令牌在 `src/index.css`）。新增 shadcn 组件：

```bash
pnpm dlx shadcn@latest add <component>     # components.json 已配置好别名与主题
```

## 架构速览

- `src-tauri/src/providers/` — 每个搜索来源一个模块（apps / clipboard / everything / files / recent / settings / sysutil / vault / web / winsys），统一产出 `SearchItem`
- `src-tauri/src/providers/mod.rs` — `SearchEngine` 汇总各源，`score_match` 统一打分（直连 → 拼音全拼 → 首字母）
- `src-tauri/src/lib.rs` — 窗口/托盘/全局热键/失焦隐藏、Everything 托管安装与自动启动、命令入口
- `src/App.tsx` — 单面板 React 组件，输入防抖调用后端 `search`，账号库保护域与表单/导入/详情页状态机
- `src/components/ui/` — shadcn/ui 组件；`src/index.css` — 设计令牌与全局动效关键帧

## 许可

[MIT](LICENSE) © zap contributors
