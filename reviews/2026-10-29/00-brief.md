# zap 十角色生产级评审 · 共享简报（2026-10-29）

## 项目是什么

zap：Windows 本地启动器，**个人开发者按生产标准维护**，v0.13.0 已发版（GitHub Releases 三件套：setup.exe + latest.json + .sig）。技术栈：Tauri 2（Rust 后端 + WebView 前端 React 19 / TypeScript / Vite）。

## 功能地图（以代码为准）

- **主面板**：全局热键呼出 → 万能搜索框。数据源：本地应用索引（含 UWP）、Everything 联动（可选）、最近文件 recent、文件搜索、web 搜索、自定义快捷方式、frecency 排序、置顶（📌 结构化字段）。
- **剪贴板历史**：文本 clip + 图片 clipimg 两 kind（线协议已统一 target=行号）；图片降采样预览面板、DIBV5 直写毫秒级复制、密码管理器来源过滤（is_from_password_manager）、图片入库走有界队列+独立 worker。
- **中转站 stage**：QQ 式全局拖拽落区条（自有 IDropTarget、四角停靠可设置、悬停展开大卡片、drop 即开面板可设置开关）、面板内固定/临时两层、合并粘贴（多选按序注入）、跨通道刷新。
- **密码库 vault**：DPAPI 加密、行内明文查看（用户取向：查看便捷优先）、30s 自动掩码、TOTP、导入导出、行内固定。
- **工具面板**：JSON 工作台（分析/格式化/树视图，单次 IPC）、文本工具（texttools）、超级面板（超长文本/JSON/图片预览）、账号解析 acctparse、系统工具 sysutil。
- **插件体系**：入口声明化（入口词 + 冲突检测）、panel.ts 面板契约（clipimg 是首个消费者）。
- **设置**：外观、直达热键四槽、拖拽落区条全套开关、配置导入导出（前后向兼容已测试锁定）。
- **更新**：useUpdater + latest.json + 签名。

## 代码规模与热点（按体积）

`lib.rs` 81.6KB（命令层+窗口管理+状态机，已知巨石）、`SettingsView.tsx` 55.6KB、`App.tsx` 53.8KB、`stagedrop.rs` 47.2KB、`providers/mod.rs` 38.9KB、`apps.rs` 36KB、`stage.rs` 32.4KB、`vault.rs` 31.3KB、`clipboard.rs` 26.8KB（另有 23 个 providers 模块、16 个分类样式文件）。测试：Rust 117（+1 smoke）、vitest 32、tsc 全绿。

## 工作区现状（评审事实源）

有 **5 个文件的未提交改动**（lib.rs / clipboard.rs / App.tsx / plugins/types.ts / types.test.ts）：clip 文本条目改按行号寻址、回贴按行号回查全文、列表不再携带内容本体过 IPC。**这是在途工作，语义完整、磁盘内容健康（UTF-8 正常）。评审一律以磁盘当前状态为准，把这笔改动当作最新事实审。**

## 已拍板的产品决策（评审必须尊重；要推翻必须给出明确论证并标注「推翻既有拍板」）

1. **交互原则**：键盘仅 调起/Esc/方向键/Enter，其余鼠标；**不新增需要记忆的组合键**。
2. **密码查看便捷优先**（内联明文，不做「每次输入主密码」式安全强化）。
3. **uTool 对标拍板**：砍 AI / 快捷命令 / OCR；保留万能输入框/搜索捷径/插件入口声明化/超级面板。
4. **国内更新双源：暂不做**。
5. 本地优先、无遥测（如你发现与事实不符，指出）。

## 已知工程纪律与历史坑（已修复，不要重复报告；但请检查是否有回归或同类未收口场景）

- 焦点丢失致 Esc 失效两类根因；**纪律：模式化键盘面一律挂 window 捕获，永不挂面板/输入框 onKeyDown**（已三次收口：表单 Esc、合并模式 Ctrl+Enter、StageView 方向键）。
- IME 卫兵（isComposing）挡选词键。
- 图标提取 SHGetFileInfoW 高并发间歇失败 → 全局串行锁 + 失败不缓存；**教训：非确定失败不得缓存、并行边界按成本模型切分**。
- 图片入库管线移出 watcher 回调线程（有界队列容量 4）。
- 决策提纯纪律：轮询/布局类判定收敛进可测纯函数（decide、corner_origin、avoid_drag_origin）。
- Windows 上用 PowerShell 改源码会 GBK 毁文件——源码修改一律用编辑工具（本次评审只读，不受影响）。

## 通用要求

- 读代码验证每个论断，**每条发现必须带 file:line 证据**。
- 生产级标准：假设这个应用明天要分发给 1000 个陌生用户、出现事故需要远程诊断。
- 不确定的事写进「诚实清单」，不要臆断。
