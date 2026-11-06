# 设计稿 · 拖拽出（Drag Out）——从 zap 直接把内容拖给任何应用

> 立项：2026-11-05 用户拍板（「拖拽出这个功能挺吸引我」）。
> 状态：**v3**——PoC 已通过（探针真机两次 COPY 落地），本版吸收 PoC 的五层实证修正；
> v2 评审档案见 `reviews/2026-11-05/`，探针 `src-tauri/examples/drag_out_probe.rs`。
> 开工等 v0.17.0 发版收工（已完成，tag 38a5d3b / Release 已发布）。
> **v3.1 修订（2026-11-06 用户裁决）**：文本条目「只给文字」——CF_UNICODETEXT 主载落地，
> 不再写 .txt 临时文件；§二 载荷表 / §三 链路 / §五 验收 2 / §六 分期按此口径更新。
> 前置阅读：stagedrop 的 OLE 经验是本功能的技术地基（`src-tauri/src/providers/stagedrop.rs`）。

## 〇、PoC 五层实证修订（2026-11-06，探针四轮迭代真机裁决）

设计稿 v2 的后端骨架有五处被探针证伪，**全部已实证修正**：

1. **`DoDragDrop` 的 `IDropSource` 参数不可为 None**——缺它返回 `CO_E_NOTINITIALIZED`（v2 未提及该参数）。
2. **线程初始化必须 `OleInitialize`，`CoInitializeEx` 不够**——只 CoInitializeEx 的线程调 DoDragDrop 同样报 `CO_E_NOTINITIALIZED`（OLE 层状态检查）。
3. **「裸线程 + DoDragDrop」被证伪**——裸线程模态循环建立后**感知不到鼠标释放**（`QueryContinueDrag` 永不触发 DROP）。拖拽源必须有 GUI 语境：**线程内建窗口 + 消息队列**。生产形态 = 专用线程 → OleInitialize → 备料 → CreateWindow（隐式）→ DoDragDrop（内部自带泵，窗口过程走 DefWindowProc 即可）→ 返回。
4. **调用前必须 `ReleaseCapture()`**——按下时源窗口（生产=WebView2）持有隐式捕获，不释放则 DoDragDrop 瞬时返回 `DRAGDROP_S_DROP`（探针 v2 第三轮实证，耗时 <50ms vs 正常 2.1s）。生产中捕获在主线程的 WebView2 窗口上，须经 `run_on_main_thread` 释放。
5. **数据对象禁止 `SHCreateDataObject(None, [绝对pidl])`**——绝对 pidl 被当相对子项用，CF_HDROP 路径无效，目标全拒绝（禁止光标，第四轮实证）。正统路径 = `SHParseDisplayName` → `SHBindToParent` 拆 parent+child → **父 `IShellFolder::GetUIObjectOf(IDataObject)`**（与 Explorer 自身拖拽源同源）。

PoC 2 裁决：**默认光标模式（不接 IDragSourceHelper）不产生 `SysDragImage`**——两次真实会话观察线程零报告，stagedrop 轮询不会被误触发；`DRAG_OUT_ACTIVE` 短路降级为防御性保留（未来接拖影助手时是硬前提）。

## 一、一句话定位

与落区条（拖**进** zap）成对：剪贴板历史 / 中转站 / 图片预览里的任何条目，**按住拖出去**，落进微信、Explorer、IDE、邮件——不再需要「先复制 → 切窗口 → Ctrl+V」三步。

## 二、交互设计

### 触发面与手势

| 面 | 手势 | 说明 |
|---|---|---|
| 主面板结果行 | 按住行 → 移动 ≈6px | 拖动即拖出；**单击仍打开**（无位移即 up，语义自然分离） |
| 中转站行 | 同上 | 死链条目（`missing`，stage_list 已算好）禁拖，**拖拽路径零 stat**（UNC 秒级，不得现查） |
| 图片预览面板画布 | 按住图片 → 移动 | **仅 fit 态生效**；zoomed 态左键拖拽 = 平移（既有手势，`setPointerCapture`），两分支按 zoom 状态互斥 |

- **mergeMode（合并粘贴勾选模式）下拖出禁用**：该模式下 click=勾选切换，拖动的 trailing click 会变成隐形勾选（意图判定 hook 加 `enabled` 参数，V2 再议翻转）。
- 行尾固定按钮、右键菜单不受影响（button 不起拖、右键不触发意图）。
- 拖出是**复制语义**：源数据（历史/中转站）零改动。
- **拖出期间面板隐藏，取消才复现**：起拖即 `win.hide()`（只隐藏，不销毁 webview、不 `emit shown`、不重新定位，前端搜索词/视图/行状态原样保留），避免浮层挡住目标应用的落点。仅 **effect=0 的取消**（Esc / 拖回原处 / 禁止目标 / 目标未收）才在原位置复现并 `set_focus()`；成功落地保持隐藏等下次呼出。`hold_panel` 同时挡住 hide 引发的 `Focused(false)`，不让失焦即隐二次收面板。早退路径（卫兵拒绝 / 备料失败）走不到隐藏，面板不会被误藏。

### 丢失单击的对策（R02-P1：trailing click 误回贴）

DOM `click` 与位移无关：mousedown+mouseup 同元素即触发。快速甩动（down→6px move→up）两种结局都必须处理：

- 结局 A（后端卫兵赢、拖拽未起）：click 照常触发 = clip 行被误回贴进前台应用。**修法：意图确认（超阈值 invoke 前）武装一次性 click 抑制器，`drag_out_begin` settle（无论成败）后解除。**
- 结局 B（DoDragDrop 赢、接管鼠标）：页面收不到 mouseup，单击丢失——与 Explorer 拖拽后不触发单击同语义，**可接受**。

### 拖出会话标志（DRAG_OUT_ACTIVE）——一个标志堵三个口

拖出进行中，`DRAG_OUT_ACTIVE` 原子量置位（生命周期 = 拖拽起止），三处消费：

1. **stagedrop 轮询短路**（R01-P1）：系统拖影窗口类名恰是 `SysDragImage`——stagedrop 靠它判定拖入会话。拖出一旦产生该窗口，stagedrop 会误判拖入、弹落区条，而 `handle_stage_drop` 无来源门控，临时文件会被误收进中转站。标志进 `decide` 的 Start 分支（比照 STAGE_VIEW_OPEN 抑制路径）+ Drop 拒绝。
2. **前端拖入通道短路**（R02-P1：自拖自拖入回环）：StageView 的 `onDragDropEvent` 在视图挂载期存活，从中转站行拖出、悬停自身列表 = 整列表降暗 +「松手放入」误提示，松手 = 文件被加回中转站。`dragOutActive`（前端标志，生命周期 = invoke promise）短路拖入监听。
3. **主面板拖回自身的边界**：主窗口 wry 兜底仅 stagedrop label 生效（lib.rs:142），但 stagedrop 窗口自有 IDropTarget 与 StageView 监听都是真实接收面——靠 1/2 短路后，拖回自身 = 静默取消，无副作用。

### 各 kind 的拖出内容

| 源 | 拖出 | OLE 载荷 | 临时文件 |
|---|---|---|---|
| `clip` 文本条目 | 文本 | **CF_UNICODETEXT（纯文字，自有 IDataObject）** | **零临时文件** |
| `clipimg` 图片条目 | PNG 全图 | CF_HDROP(→.png) | `%TEMP%\zap-drag\<内容导出名>.png` |
| `stage` 文件/文件夹 | 原件 | CF_HDROP(原路径) | **零临时文件** |
| 死链条目 / app·recent·web·url 等入口类 | 禁拖 | — | 非内容域或路径失效 |

- **临时文件名不用裸 rowid**（R01-P2）：`<id>.png` 是接收方展示名——微信会原样发出。图片用 `image-<纳秒>.png` 防同秒覆盖。
- **文本条目只给文字**（2026-11-06 用户裁决，取代 v3 的「CF_HDROP 主载 + CF_UNICODETEXT 附载」）：双载荷下微信等目标会优先收 .txt 文件，且临时文件 10s 后删除会让异步读目标扑空。改为仅报 `CF_UNICODETEXT`，松手即在编辑器 / 聊天框插入文字；代价是拖到 Explorer 不再生成 .txt 文件（验收 2 已改写）。图片与中转站仍走 CF_HDROP。
- **UIPI 权限差**（R01-P2）：zap 普通完整性拖往管理员窗口 = 光标禁止 + DROPEFFECT_NONE 静默降级，Windows 拖拽协议行为，README 说明，不做检测。

## 三、技术方案

### 核心决策：不赌 HTML5 drag，自判拖拽意图

项目已有两次绕开 wry/WebView2 默认拖拽行为的先例。本次前端自判：

```
行上 mousedown(左键, button 过滤) 记起点
  → mousemove 位移 > 6px 且仍左键按住 = 拖拽意图
  → 武装一次性 click 抑制器 + preventDefault
  → invoke("drag_out_begin", {kind, rowid})
```

**hook 公共契约**（三消费方复用）：`button===0` 过滤（VaultRowPin 等行内按钮只挡 click 没挡 mousedown，意图判定须自滤）；mouseup（6px 内=干净单击）/ pointercancel / window blur 三路清态；预备期（未确认意图）冻结 hover→active 同步防高亮漂移；`enabled` 参数（mergeMode 等场景关闭意图）。

### 后端链路

```
drag_out_begin(kind, rowid)          ← async 命令
  ├─ 卫兵：GetAsyncKeyState(VK_LBUTTON) 未按下 → 立即返回 Err（前端解抑制器恢复行样式）
  ├─ DRAG_OUT_ACTIVE 置位（在命令体内、spawn 前，同步装）
  ├─ set_hold_panel(true)（同步装）
  ├─ spawn 专用 std::thread（非 spawn_blocking：STA-on-pool 反模式 + 拖拽分钟级占池）
  │    1. 备料两段式（锁纪律）：
  │       锁内点查 owned 文本全文 / PNG BLOB（≤16MB，库上限 clipimage.rs:17）
  │       → 出锁 → 锁外写 %TEMP%\zap-drag\（39MB 过 IPC 拖慢教训同族）
  │    2. 构造 IDataObject：文本 = 自有 CF_UNICODETEXT 对象（零写盘、零 PIDL）；
  │       图片/文件 = Shell GetUIObjectOf 的 CF_HDROP
  │    3. **V1 不接 IDragSourceHelper**（无拖影预览；接了必产 SysDragImage，
  │       拖影与落区条误判的取舍 V2 再议）——光标反馈由 DoDragDrop 提供
  │    4. DoDragDrop(data_obj, DROPEFFECT_COPY)   ← 模态循环，自带 Esc 取消
  │    5. 返回 effect
  │    6. spawn 延迟 10s 删本次临时文件（删除失败留痕 zap.log，不 panic）
  ├─ oneshot channel await join → 命令返回 effect
  └─ RAII guard 释放（hold + DRAG_OUT_ACTIVE）：DoDragDrop 返回即走；
     panic="abort" 下无 unwind，幽灵 hold 残余路径由 panel.rs 既有两道兜底收口
前端 invoke resolve → 解抑制器、行恢复样式
```

要点：

- **DoDragDrop 的模态循环在专用线程自 pump**，主面板 webview 照常；hold 保证拖拽期间失焦即隐不触发（起拖即 hold、返回即释放，finally 语义由 RAII 兜底）。
- **重入**：模态循环天然挡 UI（拖拽中无法再 mousedown 另一行），`DRAG_OUT_ACTIVE` 置位兜底事件级重入。
- **并发拖出**（理论上被模态挡死）：线程侧以 DRAG_OUT_ACTIVE 为闸，已置位直接拒绝。

### 新增/改动面

| 位置 | 内容 |
|---|---|
| `src-tauri/src/providers/dragout.rs`（新） | 备料（两段式）+ IDataObject + DoDragDrop + 临时文件管理；~250 行量级 |
| `src-tauri/src/lib.rs` | `drag_out_begin` 命令 + DRAG_OUT_ACTIVE 接入 stagedrop decide/Drop 分支 |
| `src-tauri/src/providers/stagedrop.rs` | decide() Start 分支加 DRAG_OUT_ACTIVE 短路（比照 STAGE_VIEW_OPEN 模式）+ 测试 |
| 前端意图判定 hook（新，公共） | mousedown/move 意图 + click 抑制器 + 清态三路 + enabled |
| 三消费方接线 | 主列表行 / StageView 行（死链禁拖）/ 预览画布（fit 态分支） |
| 前端拖入短路 | StageView onDragDropEvent + dragOutActive 标志 |
| 无迁移、无新表、无设置项 | — |

## 四、风险与 PoC 清单（开工前半天验证）

| # | 验证点 | 若不成立的退路 |
|---|---|---|
| 1 | 独立线程跑 DoDragDrop（STA）正常接管光标与模态循环 | 必须线程化成立否则整案换骨架；OLE 每线程各自 STA，预判成立需实测 |
| 2 | **裸 IDataObject（无 IDragSourceHelper）拖出时是否产生 SysDragImage**——决定 stagedrop 短路是否实际触发 | 无论结果，DRAG_OUT_ACTIVE 短路都做（防上游/主题行为差异）；若产生且观感需要拖影，V2 评估接 helper |
| 3 | 自判 invoke 到 DoDragDrop 起跑的 IPC 延迟内左键仍按下（卫兵命中率） | 常失则备料前置：mousedown 即预备、move 超阈值即起拖 |
| 4 | hold_panel 下拖拽循环面板保持可见、结束后恢复；RAII 释放全路径 | hold 机制两条产品线验证过 |
| 5 | 拖拽模态循环期间 Esc 的 keydown 是否穿透到页面（❓R02） | 真机定；穿透则拖出期间冻结 Esc 消费方一轮 |
| 6 | 目标应用矩阵：文本目标接受 CF_UNICODETEXT；微信 / Explorer / 邮件 / IDE / 浏览器上传框 接受 CF_HDROP | CF_HDROP 是 Windows 文件拖拽通用格式，个别不收写进 README；文本条目已不再走此路径 |
| 7 | 目标延迟读文件 vs 10s 延迟删除 | 拉长 60s 或按天过期 |
| 8 | 无拖影（不接 IDragSourceHelper）的观感是否可接受 | 不可接受则 V2 接 helper + 依赖 DRAG_OUT_ACTIVE 短路兜底 |

## 五、真机验收清单

1. 主面板拖图片条目 → 微信聊天框 → 发出的是该图，文件名是内容导出名
2. 主面板拖文本条目 → 编辑器 / 微信聊天框 → **直接插入文字，不产生 .txt 文件**（2026-11-06 裁决）
3. 中转站拖文件 → 微信 → 发出原文件，源路径不动
4. **拖出过程中落区条绝不弹出**（stagedrop 短路实证）
5. **拖出过程中把同一条目拖回自身列表 → 无任何条目被加入/回贴**（拖入短路实证）
6. 快速甩动（down→6px→up）× 20 次 → 无一次误回贴/误打开（click 抑制器实证）
7. 死链条目拖不动；mergeMode 下行拖动无反应且勾选态不被误改
8. 预览面板 fit 态拖出图片；zoomed 态拖拽仍是平移
9. 单击打开 / 右键菜单 / 行尾固定 / 键盘导航全部不受影响
10. Esc 取消拖拽 → 行恢复、临时文件 10s 内被清
11. 拖出后剪贴板历史 / 中转站源数据零变化
12. 拖拽全程面板保持可见不闪烁，结束后失焦即隐恢复正常
13. 管理员窗口拖入 = 光标禁止静默取消（UIPI 预期行为，非 bug）

## 六、分期

- **V1（本批）**：主面板 clip/clipimg + 中转站文件/文件夹 + 预览面板图片（fit 态）；单条拖出；文本 CF_UNICODETEXT 主载、图片/文件 CF_HDROP；不接 IDragSourceHelper。
- **V2（另议）**：多选拖出（复用合并粘贴勾选态）、拖影助手（依赖 DRAG_OUT_ACTIVE 短路的实战数据）、文本双载荷可选项（需设置项，默认仍只给文字）、HTML 富文本载荷、mergeMode 拖出翻转。

## 附：评审档案

`reviews/2026-11-05/`：CHARTER.md（含主席预埋对抗点）· R01-backend-ole.md · R02-frontend-interaction.md。两报告总裁决均为「须修订后开工」，P1×8 已全部吸收进本 v2 稿；R02 报告文件名以 CHARTER 笔误为准更正为 R02-frontend-interaction.md。

`reviews/2026-11-06/`：**实现互验**（R01-backend-ole-impl.md · R02-frontend-interaction-impl.md），总裁决均为「须修订后 commit」，四处已落地：

- **R01-F1（P1）**：`run_on_main_thread` 是非阻塞投递——改为 mpsc 等主线程真的释放完捕获再 spawn 拖拽线程，否则主线程忙时 ReleaseCapture 晚于 DoDragDrop 间歇击穿核心路径。
- **R02-F1（P1）**：意图判定补 `closest("button")` 自滤——按行尾 📌 按钮漂移 >6px 会误起拖、且 pin 的 onClick 被 click 抑制器吞掉。
- **R01-F2（P2）**：stage 备料的 `exists()` 出 DB conn 锁再 stat（失效 UNC 可达秒级，持全局锁阻塞剪贴板）。
- **R02-F2（P2）**：意图判定补 `pointercancel` / `blur` 清态（预备期面板收起时 mouseup 收不到，监听滞留 window 误起拖）。

缓办（不阻塞 commit）：命令式压暗类在 React 重渲中可能中途丢失（纯视觉，R02-F3）；droptarget DragEnter 未受门控（用户基本不可见，R01-F3）；隐式窗口与延迟删窗口的真机依赖（R01-F4/F5，真机 PoC 一并验证）。

## 附：2026-11-06 v3.1 真机修订（两笔）

**① 文本载荷只给文字**（用户裁决）：`dragout.rs` 原 `prepare("clip")` 写临时 .txt，且 Shell `GetUIObjectOf` 数据对象只给 CF_HDROP——§二 承诺的 CF_UNICODETEXT 从没实现。改为自有 `TextDataObject`（CF_UNICODETEXT → HGLOBAL），零临时文件、零 Shell PIDL；图片 / 中转站仍走 CF_HDROP。代价：拖到 Explorer 不再生成 .txt 文件（验收 2 已改写）。`begin→entering` 由首轮 95ms 降到 1–2ms（`zap.log:1690-1698`）。

**② 「第一次拖动不生效、第二次才生效」根因（✅ 已实证，修正 R02-F2）**：`src/lib/dragOut.ts` 的 `window.addEventListener("blur", onCancel, true)` 在**捕获阶段**会收到页面内「任何元素」的 blur——blur 不冒泡但走捕获。呼出后焦点恒在搜索框（`App.tsx` 的 `zap://shown` / 多处 `inputRef.focus()`），用户第一次按结果行使 input 失焦 → `onCancel()` 把刚注册的 mousemove 监听全清 → **invoke 永不发出**（后端零 `begin` 日志）；第二次焦点已不在 input，无 blur，手势才成立。证据：`zap.log:1688-1699` 两次成功 `begin` 之间夹着两次 `shown emit` 且零 `begin`。
修法：`onBlur` 只放行元素 / Document 级 blur，仅窗口自身失焦才清态；新增模块级 `activeCleanup`，每次 mousedown 先注销上一组监听——用「新按下即清旧」替代依赖 blur 来防 R02-F2 的陈旧监听滞留。回归测试 `src/lib/__tests__/dragOut.test.ts` 四条锁住该语义（含「证明 blur 捕获阶段确实到 window」的探针，防假通过）。

**③ 拖出期间隐藏面板、取消复现**（用户 2026-11-06 追加需求）：`commands.rs::drag_out_begin` 的主线程闭包内，`DoDragDrop` 之前 `win.hide()`，结束按 `dragout::should_restore_panel(effect)`（`effect==0` 才算取消）决定是否 `show() + set_focus()`。要点：只 hide、不 `emit shown`、不重新定位，保留前端状态；`hold_panel` 挡住 hide 触发的失焦即隐；成功落地保持隐藏等下次呼出。判据看 `zap.log` 的 `dragout: panel hidden` / `dragout: cancelled, panel restored` 两行。纯函数有契约单测 `dragout.rs::tests::取消才恢复面板_落地保持隐藏`。
