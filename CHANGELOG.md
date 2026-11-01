# Changelog

格式参考 [Keep a Changelog](https://keepachangelog.com/)；版本号语义化（0.x 阶段以功能批次推进）。
每版的完整提交明细见对应 tag 的 GitHub Releases 页。

## [Unreleased]

### 功能

- **链接清洗**（文本工具）：粘贴带追踪参数的链接即识别，识别条目内联给清洗后预览，工作台「链接清洗」卡给完整可复制版本——去 `utm_*` / `share_*` 前缀与广告/分享链高频追踪键（gclid、fbclid、yclid、spm、spm_id_from、vd_source、chksm 等；小红书 xsec_token、微信 __biz 等功能性参数刻意保留）；工具内另有「文本清洗」卡：去零宽字符、弯引号转半角（无得清一律不出卡）

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
