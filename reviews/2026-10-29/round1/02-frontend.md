# 前端工程与渲染性能评审 · 第一轮（2026-10-29）

评审人领地：App.tsx 结构与重渲染、SettingsView/插件视图拆分、panel.ts 契约、vitest 覆盖、键盘/IME 面、样式体系、IPC 消费侧。
事实源：磁盘当前状态（`git status` 仅 `reviews/` 未跟踪，简报所述 5 文件在途改动在当前工作区已不见，按磁盘状态审）。全部只读，未运行 UI，仅运行了 `pnpm vitest run`（31/31 全绿，1.9s）。

---

## 1. 角色结论摘要

1. **骨架质量高于同类单体**：View 判别联合状态机（App.tsx:60-70）取代 8 个并列布尔态后，「zap://shown 复位 + 互斥」天然成立；panel.tsx 契约把二段式面板的公共生命周期（行号源/粘贴替换/复制闪烁/Esc 约定）收敛为一处，新消费者（JsonWorkbench/ToolWorkbench/ClipImagePreview）接入成本确已摊薄。IPC 消费侧 14 处 `listen` 全部带清理且统一 `then(f=>f())` 惯用法，未见泄漏；zap://shown 高频呼出有 `busyRef`/`queryRef` 双守卫（App.tsx:300-315），事件风暴下行为正确。
2. **键盘面纪律在子页面收口了，但主视图自己还留着两个同类未收口场景**（F1 右键菜单键盘面、F4 非合并态 input onKeyDown），且合并模式键盘面缺视图守卫会泄漏进设置页/中转站（F2）——三次「焦点掉 body」收口的根因并未在主视图根除。
3. **IME 卫兵覆盖不全**：isComposing 只挂在 App 合并模式（App.tsx:815）与 StageView（StageView.tsx:422）两处；全项目子页面共用的 `useEscCapture`、SuperPanel、VaultFormView Ctrl+Enter 都没有（F3/F9）——这是简报点名要查的「同类未收口」。
4. **渲染热点可控但无防线**：无 React Compiler、无 memo，query 每击键与 hover 每次 mouseenter 都全量重渲染结果列表（App.tsx:1282-1297）；当前 ≤100 行规模可接受，属于「该在结构上留好位置」而非「已经出事」。
5. **测试覆盖集中在纯契约模块，panel 契约的运行时一半零覆盖**：registry/dispatch/types/configfile/commands/password 31 条全绿，但 `usePanelSource`（panel.tsx:24-44，新消费者接入的核心）、`clampMenuPos`（ctxMenu.ts）、`codeToKeyName`（SettingsView.tsx:451）三个高价值纯函数无测试；@testing-library/react 在依赖里但组件级测试 0 条。

---

## 2. 发现清单

### F1（P1）主视图右键菜单打开后键盘面整体失效：Esc/方向键/Enter 全部不可达

**证据**：右键菜单分支挂在**输入框**的 `onKeyDown` 里（App.tsx:1033-1042，绑定见 App.tsx:1239）：

```tsx
// App.tsx:1239 —— onKeyDown 绑在 <input> 上
<input ref={inputRef} autoFocus value={query}
  onChange={(e) => setQuery(e.target.value)}
  onKeyDown={onKeyDown} ...
```

而右键菜单由**行 div 的 onContextMenu** 打开（App.tsx:1291-1297），行不是可聚焦元素——右键那一刻焦点必然从输入框掉到 body，输入框的 onKeyDown 从此收不到任何键。useEscCapture 在 main 视图不挂载；唯一能兜住它的窗口捕获（App.tsx:816-824 的 ctxMenu 分支）被 `if (!mergeMode) return;`（App.tsx:813）门死，仅合并模式生效。作者显然知道这个根因——注释写着「同 onKeyDown 口径」把它镜像进了窗口捕获——却只修了合并模式那一份。

**生产后果**：任意条目右键后，Esc 关不掉菜单、方向键/Enter 不响应（菜单开着时键盘整面哑掉），只能鼠标点 backdrop。这是 2026-10-22/10-28 两次收口的同根因第 4 例，且是主路径场景（右键是宣传语里的「更多操作」入口，App.tsx:1277 空态文案还在教用户用它）。

**建议与权衡**：把 ctxMenu 的键盘语义提出为独立 window 捕获 effect（不依赖 mergeMode），或改成 `useEscCapture(() => setCtxMenu(null), ctxMenu !== null)` 与主 Esc 互斥（VaultDetailView.tsx:119-121 同款互斥写法）。权衡：主 onKeyDown 里的 ctxMenu 分支可同步删除，逻辑只留一份；注意与 merge 捕获的注册顺序无关化（互斥条件显式化后无顺序耦合）。

**置信度**：高（事件路径纯前端可推演；未真机复验，见诚实清单）。

### F2（P1）合并模式键盘面缺视图守卫：泄漏进设置页/中转站，劫持备份弹窗的 Enter 提交

**证据**：合并键盘面的门条件只有 mergeMode（App.tsx:812-813），handler 对 Enter/Escape/方向键无条件拦截：

```ts
// App.tsx:812-815
useEffect(() => {
  if (!mergeMode) return;
  const onKey = (e: KeyboardEvent) => {
    if (e.isComposing) return;
```

而 mergeMode 的退出出口只有三处：zap://hidden（App.tsx:270-276）、进保护域（App.tsx:798-803）、toggleMerge 自身。`openSettings`（App.tsx:454-459）、`openStage`（App.tsx:328-331）、`zap://preset`="stage"（App.tsx:342-348）都不清 mergeMode。合并模式下面板仍可见齿轮按钮与直达热键，进入设置页后：

- 备份弹窗口令输入框按 Enter → window 捕获先于元素 onKeyDown 触发（App.tsx:825-832 `toggleClipSelection` + preventDefault + stopPropagation），SettingsView.tsx:1335-1345 的 Enter 提交 **永远收不到**——导出确认被静默劫持为「勾选一条看不见的剪贴板行」；
- Esc → 先被合并面吃掉（App.tsx:833-836 toggleMerge），用户第一次 Esc 感知为「没反应」；
- 设置页期间 StageView（若经直达热键进入，App.tsx:342-345）与合并面两套捕获叠加，方向键双写两个视图的 active。

**生产后果**：合并粘贴（v0.13 新功能）开着时进设置做备份导出，Enter 失效 + Esc 行为错乱；属可恢复但必然被用户撞上的交互事故，且正是「模式化键盘面」纪律要防的模式泄漏。

**建议与权衡**：最小修法是 handler 首行加 `if (view.kind !== "main") return;`（与 App.tsx:1043 主 onKeyDown 的守卫同口径）；更彻底的是把 mergeMode 并入 View 状态机（`{ kind: "main", merge: true }`）——切视图即天然互斥，与注释里「zap://shown 复位与互斥天然成立」的设计意图一致。权衡：并入 View 会动 View 类型和两处 setView，改动面稍大但消灭整类问题。

**置信度**：高。

### F3（P1）useEscCapture 缺 isComposing 卫兵：IME 合成期按 Esc 会误关表单/弹窗，丢已输入内容

**证据**：全项目子页面 Esc 都走这个 hook（App.tsx 各子视图、SettingsView.tsx:499、VaultFormView.tsx:57、VaultImportView.tsx:49、StageView.tsx:404-411），它只判 key 不判合成态（hooks/useEscCapture.ts:11-16）：

```ts
// hooks/useEscCapture.ts:11-15 —— 无 isComposing 卫兵
const onWinKey = (e: KeyboardEvent) => {
  if (e.key !== "Escape") return;
  e.preventDefault();
  e.stopPropagation();
  onEsc();
};
```

同项目的窗口捕获键盘面都加了卫兵并写明用途：「isComposing 卫兵挡 IME 选词键」（App.tsx:811、StageView.tsx:417）——说明团队已认定合成期按键会以真实 key 名到达。受影响场景全是中文输入高发区：VaultFormView 备注/标题（合成期 Esc 取消候选词 → 整个表单被 onCancel 关闭，已填字段全丢）、SettingsView 备份口令（Esc → 整个备份弹窗重置，口令白输）、VaultImportView 粘贴文本、StageView 过滤词（Esc → 先清过滤/关视图）。

**生产后果**：中文用户在账号表单里打字、按 Esc 想取消候选词 → 表单关闭、输入全丢。账号表单是低频但重输入的页面，丢失代价高；目标用户（中文 UI）命中面 100%。

**建议与权衡**：`useEscCapture` 首行加 `if (e.isComposing) return;`——一处修，六处消费点全受益，且与既有两处卫兵口径对齐。VaultFormView 的 Ctrl+Enter 捕获（VaultFormView.tsx:60-70）与 SuperPanel（F9）同理补齐。权衡：若真机验证发现 WebView2 合成期 Esc 的 `e.key` 是 "Process"（则 hook 已被 key 判断挡住），本条降级为防御性对齐——但加上零成本，且能锁死各输入法/版本差异。

**置信度**：中（事件形态依赖平台/输入法，未真机验证；按团队自己的卫兵先例推断成立）。

### F4（P2）非合并态的主面板导航键仍挂 input onKeyDown：点击行后面板键盘面哑掉

**证据**：主视图方向键/Enter/Esc 在 App.tsx:1033-1107 且绑在输入框（1239）。点击任意结果行（行 div 无 tabIndex）焦点即落 body；此后直到点回输入框，方向键/Enter/Esc 全部无响应。StageView 对同构场景已按纪律做了「预防性收口」并写明根因（StageView.tsx:413-417），主视图没做。可达场景：打开失败（App.tsx:406-408 toast 报错面板不收）、kind=info 条目（routeItem → none，App.tsx:964-965）。

**生产后果**：「点了一下 → 面板没关 → 想用键盘 → 没反应」的体验断层；用户可自行点回输入框恢复，故降为 P2。

**建议与权衡**：两条路线任选：(a) 主视图导航键整体迁 window 捕获（StageView.tsx:420-437 同款，含 isComposing 与 ctxMenu 守卫），输入框 onKeyDown 删除；(b) 最小改动——行 onClick/面板容器 onMouseDown 兜底 `inputRef.current?.focus()`。(a) 一次收口整类问题、与既有纪律一致；(b) 改动最小但留着一个「为什么这里要抢焦点」的补丁。

**置信度**：高（路径与 StageView 注释自证）。

### F5（P2）ctxMenu 状态跨 hidden/shown 残留：下次呼出菜单残影叠在旧坐标

**证据**：zap://hidden（App.tsx:269-280）清 query/active/mergeMode/mergeSel，**不清 ctxMenu**；zap://shown（App.tsx:283-316）也不清。ctxMenu 只在菜单自身交互里复位（App.tsx:1037-1041、1335-1357）。呼出热键在菜单开着时收面板 → 下次呼出时旧菜单（含全屏 backdrop，App.tsx:1333-1340）原样回来。

**生产后果**：呼出后第一眼是一个来历不明的右键菜单盖在列表上；须点一下才消失。低频（菜单开着按热键收面板）但视觉事故感强。

**建议与权衡**：shown 监听里补 `setCtxMenu(null)`（与 setView 复位同一处，一行）；hidden 里加亦可但 shown 是权威复位点。权衡：无。

**置信度**：高（前端状态推演；Rust 侧 hide 是否先发 hidden 未验，见诚实清单）。

### F6（P2）结果列表无 memo：每击键 + 每次 hover 全量重渲染；key 带索引导致重排整行重挂

**证据**：AppView 单组件渲染全列表，行内联闭包（App.tsx:1289-1297）：

```tsx
// App.tsx:1286-1290 —— 索引入 key + hover 即 setState
<div key={it.id + String(i)}
  ref={i === active ? activeItemRef : undefined}
  className={"zap-item" + (i === active ? " active" : "")}
  onMouseEnter={() => setActive(i)}
```

query 每击键 → AppView 重渲染 → N 行全量 reconcile；鼠标扫过列表每跨一行一次 setState → 同样全量。`key = id + String(i)` 使 repullClips（置顶后重拉，App.tsx:725-734）里换位的条目 key 变化 → 整行卸载重挂、`<img class="zap-ico">` 重新发起加载（同 URL 有缓存，仍有闪帧窗口）。

**生产后果**：当前规模（剪贴板 100/500 档 + 搜索结果几十行）在 WebView2 上大概率无感；clipMax=5000 档 + 图片行 + 低端机是可见卡顿的上界。属结构性欠账而非现行事故。

**建议与权衡**：抽 `ResultRow` 为 `React.memo` 组件，行闭包改为容器级 `onMouseEnter` 读 `data-index`；key 改为 id（id 理论唯一，clipRowId 契约已锁行号语义，App.tsx:1282 的 `id+i` 防重复键可以只在确有重复 id 的 kind 上启用）。权衡：改动集中在列表渲染段，收益随 clipMax 增长；不做也可，但建议至少把 key 里的索引去掉（一行改动，消除重挂闪帧）。

**置信度**：中高（渲染成本是静态推演，未做 profiling）。

### F7（P2）品牌琥珀色 7 处硬编码绕过 token：浅色主题下不随 --primary

**证据**：tokens.css 声明「唯一的取色/取面处」（tokens.css:1-5），浅色主题 primary 是 #d97706（tokens.css:87），但组件 CSS 里有 7 处写死深色主题的琥珀：panel.css:62（`linear-gradient(145deg, var(--zap-warn), #d97706)` 置顶徽标）、panel.css:70（`color: #1a1205`）、panel.css:82（`caret-color: #f59e0b`）、results.css:64（active 指示条 `background: #f59e0b`）、results.css:162（置顶图钉 `color: #f59e0b`）、stage.css:126（`color: #f87171`——浅色下 --zap-err 是 #dc2626）、imgpreview.css:62（`color: #fff`）。

**生产后果**：浅色主题下置顶图钉/active 条仍是深色系琥珀，对比度与体系一致性都打折；不是错，是「token 纪律的破窗」。其余 rgba() 命中（footer/imgpreview/stage 等 ~40 处）多为辉光阴影，可接受。

**建议与权衡**：把这 7 处换成 `var(--primary)` / `var(--zap-err)`（或新增 `--zap-brand` 单一 token 收口品牌琥珀），一处换主题全量生效。权衡：纯机械替换，建议随手做；别为此扩 token 面。

**置信度**：高（静态审读；浅色实际观感未跑浏览器比对）。

### F8（P2）refreshAppearance 在所有非 settings 视图切换都重拉 get_app_settings

**证据**：App.tsx:232-234：

```ts
// App.tsx:232-234 —— 条件是「非 settings」而非「从 settings 退出」
useEffect(() => {
  if (view.kind !== "settings") refreshAppearance();
}, [view.kind, refreshAppearance]);
```

意图是「设置页关闭后重拉」（注释 214），实际 main→stage→main→detail 每次切换都发一次 IPC；设置页内改主题的即时预览（SettingsView.tsx:923-927）已本地 applyAppearance，只有「保存前 Esc 放弃」需要回拉。

**生产后果**：无功能危害，纯 IPC 浪费；每次进出中转站/详情页多一次命令往返。

**建议与权衡**：条件改为「上一个 view.kind 是 settings 且本次不是」，用 ref 记 prev；或干脆由 closeSettings 显式调 refreshAppearance（App.tsx:461-465 已是统一出口）。权衡：一行改 closeSettings 最简。

**置信度**：高。

### F9（P2）SuperPanel 键盘面 bubble 挂载且无 isComposing：与「window 捕获」纪律不一致

**证据**：SuperPanel.tsx:68 `window.addEventListener("keydown", onKey)`——**未传 capture=true**，也无 isComposing 卫兵；对比同类键盘面全部捕获 + 卫兵（App.tsx:848、StageView.tsx:435）。当前面板无输入框，Enter 的 preventDefault 也确能压掉聚焦按钮的原生 click（注释自证 SuperPanel.tsx:50-52），所以暂无害。

**生产后果**：当下为零；未来面板内加任何输入框或任何组件 stopPropagation，键盘面静默失效——纪律注释（panel.tsx:6-7）明说这是栽过三次的坑。

**建议与权衡**：补 `true` 第三参与 isComposing 卫兵，两行改动，把最后一个键盘面拉齐到同一纪律口径。权衡：无。

**置信度**：高。

### F10（P2）测试缺口：panel 契约运行时、两个纯函数、merge 键盘语义均无锁定

**证据**：31 条测试分布（`pnpm vitest run` 实测）：configfile 6 / commands 7 / registry 6 / dispatch 5 / password 4 / types 2 / appearance 1。以下三处高价值目标零覆盖：

- `usePanelSource`（panel.tsx:24-44）：text 与 clipRow 互斥（text 优先）、clip_row 失败置 loadErr、cancelled 守卫——这是「新消费者接入成本」承诺的运行时一半，且 commands.test.ts:5 已有 invoke mock 模板可复制；
- `clampMenuPos`（ctxMenu.ts:13-18）：纯函数，PANEL_W/H 与 lib.rs、tauri.conf.json **三处手工同步**（ctxMenu.ts:3 注释自认），无回归锁；
- `codeToKeyName`（SettingsView.tsx:451-456）：热键录制正确性的核心映射（物理键码→键名，注释强调「中文输入法下 e.key 常为 Process」），内联未导出、零测试。

**生产后果**：panel 契约改动（如再进消费者）无测试兜底；菜单尺寸常量漂移（改 CSS 条目高不同步 ITEM_H=30）会静默错位。

**建议与权衡**：三个纯函数/伪纯函数各补一组 ≤6 条断言；merge 键盘面若按 F2 并入 View，顺手把「merge 模式下的按键决策」提为纯 `decide`（对齐 Rust 侧「决策提纯」纪律，App.tsx:948 注释已有此先例）。权衡：均为既有 mock 设施内的增量，无新依赖。

**置信度**：高。

---

## 3. 前端类功能或流程更改建议

1. **键盘面一次收口**（对应 F1/F2/F3/F4/F9）：useEscCapture 加 isComposing；ctxMenu 键盘语义提为独立捕获（互斥条件显式化）；merge 键盘面加 view 守卫或并入 View 状态机；SuperPanel 补 capture+卫兵。建议作为「键盘面纪律 v2」一次 PR 完成，并在 panel.tsx:6-7 的契约注释里把四条口径（window 捕获 / isComposing / 互斥 enabled / 每渲染重挂）写全，让下个消费者照抄。
2. **merge 模式并入 View 状态机**：`{ kind: "main"; merge: boolean }` 替代并列布尔（App.tsx:175-176），切视图互斥天然成立，App.tsx:798-803 的守卫 effect 与 F2 的泄漏一并消失。
3. **主面板补焦点兜底**：行点击/面板空白 mousedown 后 refocus 输入框（F4 路线 b），或导航键迁 window 捕获（路线 a）——二选一，别两个都做。
4. **测试补齐三件套**：usePanelSource（mock invoke，锁 text 优先/loadErr/cancelled）、clampMenuPos（锁边界夹紧与常量口径）、codeToKeyName（提取导出后锁 Key/Digit/F/Space 映射与「必须带修饰键」约定）。
5. **建立「键盘面 grep 自检」到 CI 的流程**：把本评审用的两条 grep（`onKeyDown` 用法清单、`Escape|isComposing` 覆盖清单）固化为 lint 规则或 CI 检查脚本——onKeyDown 只允许出现在「输入框复制拦截」类（VaultRevealOverlay.tsx:63 是正当用法）与已收口文件白名单里，新文件越界即红。比靠注释纪律防回归可靠。

---

## 4. 诚实清单

- **F3 的 IME 事件形态未真机验证**：WebView2（Chromium）+ 中文输入法下，合成期 Esc 的 keydown 究竟是 key="Escape"+isComposing=true 还是 key="Process"，我无法在只读评审中确认。团队在 App.tsx:811/StageView.tsx:417 主动加卫兵是本条成立的最强旁证，但请按 F3 的验证步骤真机确认后再定级。
- **简报称 vitest 32 条，盘面实测 31 条**（7 文件全绿）。types.test.ts 当前 2 条；差异可能来自在途改动的提交/回退，未深究。
- **git status 显示工作区仅 reviews/ 未跟踪**：简报所述 5 文件未提交改动在当前盘面已不存在（应已提交或回退）。本报告全部以磁盘状态为据。
- **F1/F2/F5 的可达性基于前端事件路径推演**，未启动应用复现；Rust 侧 hide/show 事件的确切发射时序（是否保证 hidden 先于 shown、hold 期间 show 是否发生）未读 src-tauri 侧代码。
- **渲染性能未做实测 profiling**（F6）：无 Performance 面板数据，规模上界（clipMax=5000）下的实际帧率是推断。
- **样式仅静态审读**：浅色主题 7 处硬编码琥珀的实际观感（F7）未跑浏览器比对；16 个 CSS 文件中 base/tokens/results/panel/ctx-menu 精读，其余抽查。
- **SettingsView.tsx 1416 行只精读了状态/热键/保存/备份弹窗段**（1-640、1290-1416），中部渲染段（969-1290，更新/插件/捷径分区）按行扫描未逐行；ToolWorkbench.tsx、VaultRowPin.tsx、utils.ts 抽查未逐行。
- **未评估高 DPI/多显示器缩放**对 clampMenuPos 固定常量（720×480）的影响——若系统缩放改变 WebView CSS 像素口径需另行验证。
- **未审 Tauri 权限/capability 与 IPC 注入面**（`invoke` 参数是否可能被搜索词污染）——那是后端/安全评审角色的领地，前端侧只确认了所有 invoke 参数均来自后端列表数据或受控行号（clipRowId NaN 守卫已在 types.test.ts 锁定）。
