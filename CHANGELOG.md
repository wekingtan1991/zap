# Changelog

格式参考 [Keep a Changelog](https://keepachangelog.com/)；版本号语义化（0.x 阶段以功能批次推进）。
每版的完整提交明细见对应 tag 的 GitHub Releases 页。

## [Unreleased]

## [0.18.0] — 2026-11-06

### 交互：主面板工具 Dock 常驻

- **入口迁出结果列表**：插件入口行原先混在列表里（空面板组装与「无强命中」兜底两处 append），空态呼出时占掉 5–6 行黄金地产（最近剪贴板与常用应用被挤走）。现改为结果区与底栏之间的**常驻 Dock 细条**，列表回归纯内容，空态与搜索态位置恒定不闪变
- **入口与内容按数据来源分离**：后端新增 `tool_entries` 命令，只收启用插件的 `fallback_entry()`，并附带 `PluginMeta.title/entries`（chip 短名与悬停提示的入口词都随插件声明走，前端不硬编码）；`engine.search` 两处 append 与 `has_strong` 兜底判定一并删除。**不按 kind 拆分**——`probe`/`detect` 产出的 json/tool/port/info 是内容动作行，原样留在列表
- **Dock 行为**：chip 图标色复用 badge variant 语义色（含浅色主题覆盖，不新造色值）；点击走既有 `runItem` 分发（stage 的 `openStage`、cmd 的 `runCommand`、json/tool/port 工作台 activate 全在其上）；停用插件不出现，超过 8 个收进「⋯」溢出菜单；全部按钮 `tabIndex=-1` 且不绑 keydown，**零新增键盘组合键**
- **底栏**撤掉「json/tool 搜索工具」提示（入口已由常驻 Dock 承担），问号帮助的次级操作说明保留

### 新功能：拖拽出（条目按住拖给任意应用）

- **前端自判拖拽意图**（mousedown + 6px 阈值），不赌 HTML5 drag；click 抑制器防快速甩动误回贴，窗口级 blur / pointercancel 清态，新按下先清旧监听
- **Rust 主线程 DoDragDrop**（`IDropSource` + `OleInitialize` + `ReleaseCapture`）；文本走自有 `IDataObject` 的 CF_UNICODETEXT 纯文字载荷（编辑器/聊天框松手即插入文字，不产生临时文件），图片与中转站文件走 Shell CF_HDROP
- **会话门**：`DRAG_OUT_ACTIVE` 短路 stagedrop 轮询、自有 `IDropTarget` 与前端拖入回环
- **面板生命周期**：拖出期间隐藏面板，取消（effect=0）原位复现，成功落地保持隐藏
- 同步 `docs/design-drag-out.md` v3.1、PoC 探针（`drag_out_probe`）与 4 条 `dragOut` 回归测试（含「blur 捕获阶段确实会到达 window」的探针，防假通过）

### 修复：Everything 托管链（2026-11-06 真机实证）

- **启用入口卡死（根因）**：`launch_elevated()` 用 `Start-Process -Verb RunAs -Wait`，而 `-Wait` 等的是提权启动的**进程树**——批处理里 `start` 出来的 `Everything.exe` 是常驻后台进程，于是外层 shell 永不退出、`everything_setup` 命令挂死，前端随之锁死（busy 挡住所有条目、hold 让面板失焦不隐、占位行跨呼出残留）。改为只发起提权不等待，就绪检测交既有 `wait_running(20s)` 轮询；UAC 取消改用 `ErrorActionPreference=Stop` + `try/catch exit 1` 保住非零退出码判定
- **前端不再有死状态**：hold 与 busy 一律走 `finally` 释放；面板隐藏即视为放弃等待（在途回执不回写列表、代次作废），下次呼出能正常重新搜索
- **hold 窗口精确化**：后端在 UAC 阶段结束（授权或取消）即发事件，前端收到立刻释放 hold——hold 从此只守 UAC 抢焦点那几秒，下载与轮询期间面板可正常失焦即隐
- **下载预算封顶**：官网解析页 10s、单 zip 30s、整轮 90s；且只下载哈希表内的版本（表外版本校验必拒，先下再拒纯浪费流量与时间），表外版本单列出来提示手动安装
- **状态映射不再误导**：`running_no_http`（Everything 已起、只是 HTTP 未就绪）此前被当作失败并显示天书文案，现单列一条说明现状；占位文案按状态分支——已安装说「正在启动已就绪的 Everything」，未安装才说「正在下载约 2MB」
- **版本解锁**：哈希表补 `1.4.1.1032`（官网稳定版此前已到 1032，表里只有 1028，用户永远装不上最新版），哈希取自官网同名 `.sha256` 文件并与既有条目逐字符核对；内置兜底版本表同步精简为表内版本
- **链路可观测**：`setup()`/`download()` 补关键节点日志（开始 / 下载中 / 校验通过 / 哈希不匹配 / HTTP 错误 / 跳过表外版本 / 提权发起与失败 / 已就绪 / HTTP 未就绪）——此前整条托管链在 zap.log 里零记录

### 视觉

- **结果行类型胶囊常驻行首**：图标与胶囊由二选一改为并存（kindUi 胶囊 48px 对齐列 + 图标跟排），剪贴板等无图标行视觉不变，统一分类辨识

### 发布工程

- 四处版本号同步 0.17.0 → 0.18.0（package.json / tauri.conf.json / Cargo.toml / Cargo.lock）

## [0.17.0] — 2026-11-05

### 无障碍与交互（2026-11-05 续批）

- **读屏与键盘**：主面板结果列表与中转站列表补 `combobox`/`listbox`/`option` 语义及 `aria-activedescendant`，免焦点键盘面对屏幕阅读器可见；三个自绘弹窗补 `aria-labelledby` 与焦点陷阱（初焦 / Tab 困笼 / 关闭回焦）；右键菜单补 `menu`/`menuitem`、打开聚焦首项、菜单内方向键循环、Esc 回焦输入框。顺路收口分区标题 heading、更新进度条 progressbar、错误 role=alert、Select 关联 Label、合并计数 aria-live 等。零键盘面行为改动、零新增组合键
- **视觉与措辞**：logo 磁贴品牌色收进 `--zap-logo-*` 令牌（浅色主题保留琥珀渐变，近黑图标对比度过 3:1）；用户可见「阅后即焚」改「延时清空」（实为延时清除、复制新内容即放弃），中转站反馈「已置顶」改「已固定」，全站同义词按域维持现状

### 评审修复（2026-11-04 二十人多轮评审 + Round 2 互验落地批）

- **正确性与隐私**：账号库复制类动作失败即止步——回填/取图任一失败不再照旧注入 Ctrl+V 把旧剪贴板内容贴进前台窗口（`paste_bg`/图片分支两处）；详情页「网址」行一键复制此前并未复制（前端假亮 ✓），新增 `url_copy` 动作真写剪贴板；账号库删除同步失效密码集（已删密码不再永久抑制剪贴板入库）；密码防护改逐行解密、单行密文损坏不再放大成整个防护集为空；watcher 的密码历史清除/入库/保留淘汰三处静默失败留痕 zap.log
- **稳定性**：数据库迁移失败不再静默继续（上传播走坏库隔离重建）；打开损坏/恶意备份文件（非 12 字节 nonce）报错而非 panic=abort 崩进程；备份导出改两段式——锁内只捞密文行，DPAPI 解密与 60 万轮 PBKDF2 全部移出连接锁（导出期间不再挡剪贴板收录与击键搜索）；`OpenProcess` 句柄泄漏修复（常驻内核句柄无界累积）；三处 `CoInitializeEx` 按 COM 契约配对收尾（S_OK 与 S_FALSE）；三个窗口 Alt+F4 改隐藏不再制造半死态
- **线协议**：`vault_get` 位置七元组改 `VaultRecord` 命名字段结构体（字段调换不再可能把密码静默填成网址），前端按键取值带类型
- **前端反馈链**：呼出首帧先出列表再探测 Everything（未装用户不再被端口探测扣住第一帧）；超级面板动作 ✓ 改执行成功后点亮、失败 toast（该窗口补挂反馈通道）；快瞄浮层 30 秒计时器不再随父级渲染无限重置
- **发布工程**：release.yml 出包前与 CI 同面重跑 fmt/clippy/vitest/check:kb/版本一致性（tag 可绕过 CI 直打，必须自设门）；ci.yml 并发取消；pre-push 补 Rust 单测；`.gitignore` 收编 `.tmp-*`
- **文档**：README 架构速览对齐现状、隐私表述如实化（不上传个人数据，外联三处逐条列出）、功能表补中转站；SECURITY.md 同步；`components.json` css 路径修正；发版清单补私钥公钥配对手验步骤
- **测试基建**：vault 并行内存库与全局形态缓存的跨测试污染串行化；写真剪贴板断言三级应对（瞬时重试/系统级卡死跳过）；settings 白名单安全闸门直接测试；`AppSettingsDto` serde 键集快照测试；备份恢复恶意 nonce 回归测试
- **缓办批与拍板项（11-05 续批）**：可执行类图标缓存键入 mtime + 2048 上限整体清空；`stage commit_add` 事务失败留痕并回滚整批；`vault_last_export_at` / `everything_ensure` 取设置压进阻塞线程，不再占用 tokio worker；透明效果 toggle 只做即时预览、落库随 `save_settings` 且失败回退；开机自启勾选未被用户改动时不再回写注册表；文件夹条目点击改 explorer 直开（原 open crate 目录分支在本机静默失效）

### 安全与工程

- **敏感信息治理**：测试夹具统一为 RFC 2606 保留域、RFC 5737 文档网段、138/139 测试号段等常用范例，并加「禁止贴真实账号文本」注释；新增 `check:secrets` 通用规则扫描（14 类凭证特征 + 常用范例白名单）接入 pre-push；`.gitignore` 补齐本地数据库、凭证文件、覆盖率与打包/签名产物，评审档案目录不再误入库
- **依赖与 CI**：`pnpm/action-setup` 4.0.0 → 6.1.0（消除 Node.js 20 弃用告警）；`@vitejs/plugin-react` 6.1.2 → 6.2.0；移除四个零引用 npm 包装包；dependabot 对 0.x 依赖的跨 minor 破坏性跨度改为人工评估，同 minor 的 patch 仍自动提

## [0.16.0] — 2026-11-02

### 功能

- **图片预览缩放**（中转站图片预览面板）：滚轮以光标为锚点缩放（光标下图像点不动）、操作条尾 −/百分比/＋（百分比标签即「回适应」重置钮）、放大后左键拖拽平移（边界 no-gap 夹紧——图不出舞台、不留缝）、双击回完全展示；倍率以完全展示为 1 倍，下限=完全展示（此时恒居中、不可平移）、上限 8×；缩放数学抽纯函数 `clipimg/zoom.ts` 配 8 项单测
- **链接清洗**（文本工具）：粘贴带追踪参数的链接即识别，识别条目内联给清洗后预览，工作台「链接清洗」卡给完整可复制版本——去 `utm_*` / `share_*` 前缀与广告/分享链高频追踪键（gclid、fbclid、yclid、spm、spm_id_from、vd_source、chksm 等；小红书 xsec_token、微信 __biz 等功能性参数刻意保留）；工具内另有「文本清洗」卡：去零宽字符、弯引号转半角（无得清一律不出卡）

### 修复

- 设置页保存成功后 Esc 仍弹「未保存」确认：脏态基线 useRef 写入不触发渲染、dirty 渲染期派生在零 setState 的干净保存上全 bail out 吃旧帧恒脏——基线改 state 驱动重渲
- UI 排版专项审计 P1×3+P2×4 修复批：硬编码色收令牌（琥珀高亮、stage 失效标签红）、灰阶三档与字号五档收敛成文 tokens.css、原生 button focus ring 兜底、px 溢出与圆角收敛

## [0.15.0] — 2026-11-01

### 依赖与发布工程

- 依赖批量升级（dependabot 九 PR 内容合入八）：Actions 钉版 SHA 升级（checkout v7.0.1 / setup-node v7.1.0 / upload-artifact v7.0.2）；**Cargo 加密栈整族对齐 digest 0.11**（sha1/sha2/hmac/md-5/pbkdf2——digest 0.10 与 0.11 的 trait 生态不互通，单包 PR 不可合必须整族同升，TOTP 官方向量与备份加解密回环测试验证）；arboard 3.7 / clipboard-master 4.1 与 tauri 插件四件；npm 组十二件（react 19.3.1 / vite 8.3.5 / @tauri-apps api·cli 2.12.4 / lucide-react 等）；windows-core 0.62→0.100 跨版刻意未合（牵动全部 Win32 调用，单独评估）
- 键盘面纪律自检（`pnpm check:kb`）接线 pre-push 钩子，白名单随备份域迁移更新——消除「CI 在 master 红、本地全绿」的门槛不对称

### 界面

- 设置页十二分区由分隔线改轻卡片：`--zap-w-04` 底 + `--zap-w-08` 描边 + 12px 圆角（与 vault 行卡同参数族），分区扫视定位更清晰；备份分区一并收编进共享 Section 壳
- 设置页保存成功反馈改全局 toast（与全应用反馈形式统一，行内「已保存 ✓」退役；字段级校验错误保持行内）

## [0.14.0] — 2026-11-01

### 评审修复（2026-10-29 十角色三轮交叉评审落地批，P0×3 + P1×20 + P2×18 全部落地）

- **进程软着陆**：panic hook 死因留痕（zap.log）；坏库隔离重建继续启动（不再「手删库连账号库一起丢」）；账号库备份提醒（上次导出时间 + 30 天警示）
- **首启引导**：首装自动呼出一次面板；托盘 tooltip 动态化（正常态显示呼出热键 / 热键被占用时告警并可点击呼出）
- **分发信任**：Release 附安装包 SHA256 并写入 release notes
- **键盘面纪律 v2**：右键菜单键盘面迁窗口捕获（第四次同根因收口）、合并键盘面补视图守卫、全项目 Esc 面板补 IME 卫兵、CI 新增键盘面自检（`pnpm check:kb`）
- **存储收口**：文本单条 1MB 上限、搜索评分改读物化列（head/nl_count，迁移器 v2）、图片 256MB 字节预算双阈值淘汰、synchronous=NORMAL + 退出 WAL checkpoint、保留条数后端 clamp
- **主线程写路径**：open_item 文本两段式、启动计数挪 spawn_blocking、save_settings 单事务、busy_timeout 显式 1500ms
- **安全**：剪贴板入库前与账号库密码集比对（浏览器密码管理器复制的密码不再入库）+ 单条删除/清空补救；用户名/备注/超级面板复制统一切受保护通道（不进 Win+V）；焚除窗口内快速粘贴跳过；Everything HTTP 服务加 Basic Auth + seed_ini 改只补缺（不再复活用户手动关闭的服务）
- **可靠性**：关键失败落 zap.log（[E]/[W] 级别 + 启动诊断行，capture.log 并入）；剪贴板监听线程自愈 respawn（3 次退避）
- **交互**：合并模式非 clip 行禁用态与入口收敛；工具面板 Ctrl+V 改常驻监听（空态指引恒真）；账号库行右键菜单 + 详情页「打开网址」按钮；超级面板恢复改 DIBV5 直写（大图不再秒级阻塞）
- **发布工程**：user_version schema 迁移器；四处版本一致性校验（`node scripts/verify-version.mjs`，接线 pre-push 与 CI）；更新检查 24h 间隔重查；依赖 action 全部钉 commit SHA + dependabot 三生态周更

### 质量审计（2026-10-31 整洁/复用/大文件/性能四维，A–G 七批 20 提交）

- **整洁/复用**：vault stub 与死导出焚除、providers/util 收编 now_ms/human_size/wide 三族重复、直达热键槽位表合一（单一事实源）、clipimage/stage 线协议结构体化
- **大文件拆分**：providers/mod.rs 1116→60（matching/engine/settings_owner 三件）、clipboard.rs 1090→413（db/winclip/watcher/clipboard 四层归位）、stagedrop COM 拆 droptarget、apps 拆 icons/lnk、commands 874→828（blocking! 宏收编 29 处样板）
- **前端拆分**：SettingsView 1658→280（备份域内聚 + 热键录制 hook 化 + 九分区组件化）、App.tsx 1386→1073（PanelFooter/ResultsContextMenu/useMergeMode/ResultsList 四件）
- **性能**：vault 搜索打分形态缓存（写后失效，契约测试锁定）+ 五处每击键热查询 prepare_cached

### 修复

- 设置页脏态保护假阳性：未改任何项 Esc 也弹「放弃更改」（基线与对比的序列化键名漂移致恒脏）；保存成功后不置灰；「返回」按钮绕过未保存确认
- 发布工程：pnpm/action-setup 钉版引用了不可解析的 SHA，致 CI/Release 全部 run「Set up job」秒败（本版发布过程中发现并修复）

## [0.13.0] — 2026-10-28

- 拖拽落区即开中转站面板（设置可关）；落区条停靠角四角可选；细条高度预算 36→44 去扁平
- 落区条竞态修复（DragEnter 与轮询跨线程）；视图展开期间条子抑制
- 图标高并发提取间歇失败修复（全局串行锁 + 失败不缓存，可执行类按路径缓存）
- 图片预览操作条鼠标跟随高亮；drop→开面板移交提前消灭共存窗

## [0.12.0] — 2026-10-28

- 应用配置导出/导入（ConfigFile JSON，账号库走 .zapvault 口令备份）
- 中转站体验批：跨通道刷新/视图内拖入反馈/双击防抖/固定临时分隔线/失败反馈等
- 剪贴板多条合并粘贴（点选保序，Ctrl+Enter 执行）及三轮实测 bug 修复（行号寻址契约、键盘面窗口捕获、执行后不退模式）

## [0.11.0] — 2026-10-27

- 中转站 V1：全局拖拽落区条（QQ 式）、搜索结果右键放入、固定/临时两层、撤销
- 全面审查修复 14 项；中转站评审修复 8 项；插件兜底入口清单与入口词冲突检测
- 样式文件化重构（index.css+App.css → src/styles/ 分类 16 文件，零行为变更）
