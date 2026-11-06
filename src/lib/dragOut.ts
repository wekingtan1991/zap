/// 拖拽出（docs/design-drag-out.md v3.1）前端侧：意图判定 + click 抑制 + 会话标志
///
/// 不赌 HTML5 drag（wry/WebView2 拖拽默认行为已被坑过两次的既定纪律）：
/// mousedown 记起点 → window mousemove 位移 >6px 且左键仍按住 = 拖拽意图，
/// 立即 invoke drag_out_begin（实时事件，出发时左键必然还按着——比等 dragstart
/// 更早更稳，时序竞态从根上消掉）。
import { invoke } from "@tauri-apps/api/core";

/// 拖出会话进行中（生命周期 = invoke promise，与后端 DoDragDrop 模态循环同窗）。
/// StageView 的拖入监听以此短路「自拖自拖入」回环（R02-P1）
let sessionPending = false;

export function isDragOutActive(): boolean {
  return sessionPending;
}

/// 一次性 click 抑制器：意图确认时武装、被触发或 invoke settle 时解除。
/// DOM click 与位移无关——快速甩动（down→6px→up）若被后端卫兵抢先拒绝，
/// trailing click 会按单击语义误回贴/误打开（R02-P1）
let suppress: ((e: MouseEvent) => void) | null = null;

function armClickSuppress(): void {
  if (suppress) return;
  suppress = (e: MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
    releaseClickSuppress();
  };
  document.addEventListener("click", suppress, true);
}

function releaseClickSuppress(): void {
  if (!suppress) return;
  document.removeEventListener("click", suppress, true);
  suppress = null;
}

/// 预备期监听句柄（模块级）：下一次 mousedown 先注销上一组。
/// 面板在预备期因 Esc/收起导致 mouseup 丢失时，旧监听会滞留在 window 上，
/// 带着陈旧 sx/sy 在新一次手势里误判 >6px 误起拖（互验 R02-F2）。
/// 用「新按下即清旧」替代依赖 blur 清态：既不误起拖，也不误杀当前手势。
let activeCleanup: (() => void) | null = null;

/// 意图判定入口（消费方行 onMouseDown 调用）：
/// - 仅左键；已有会话时忽略
/// - 位移阈值 6px（与探针一致）；小于阈值松手 = 干净单击，无任何副作用
/// - 确认意图后：源行压暗（视觉反馈「这份正在被拖走」）+ 武装 click 抑制器
///   + invoke；settle（无论成败）统一还原——后端失败/取消也走这条路
export function startDragOut(
  e: {
    button: number;
    clientX: number;
    clientY: number;
    target: EventTarget | null;
    currentTarget: EventTarget | null;
  },
  kind: "clip" | "clipimg" | "stage",
  rowid: number
): void {
  if (e.button !== 0 || sessionPending) return;
  // 起点落在行尾按钮（📌 固定等）上不拖出：pin 只在 onClick stopPropagation、
  // 不拦 mousedown，冒泡到行的意图判定会让「按 pin + 漂移 >6px」误起拖，
  // 且 pin 的 onClick 反被 click 抑制器吞掉（互验 R02-F1）
  if (e.target instanceof Element && e.target.closest("button")) return;
  const row = e.currentTarget;
  if (!(row instanceof HTMLElement)) return;
  // 注销上一组预备期监听（含「面板收起时 mouseup 丢失」的陈旧监听）
  activeCleanup?.();
  const sx = e.clientX;
  const sy = e.clientY;
  let started = false;

  const cleanup = () => {
    window.removeEventListener("mousemove", onMove, true);
    window.removeEventListener("mouseup", onUp, true);
    window.removeEventListener("pointercancel", onCancel, true);
    window.removeEventListener("blur", onBlur, true);
    if (activeCleanup === cleanup) activeCleanup = null;
  };
  const onMove = (ev: MouseEvent) => {
    if (started) return;
    const dx = ev.clientX - sx;
    const dy = ev.clientY - sy;
    if (dx * dx + dy * dy <= 36) return;
    started = true;
    cleanup();
    sessionPending = true;
    row.classList.add("zap-drag-out-src");
    armClickSuppress();
    invoke<{ effect: number }>("drag_out_begin", { kind, rowid })
      .catch(() => {
        // 取消（Esc/拖回原处/卫兵拒绝）是正常路径，静默；真故障有后端 zap.log
      })
      .finally(() => {
        sessionPending = false;
        row.classList.remove("zap-drag-out-src");
        releaseClickSuppress();
      });
  };
  const onUp = () => {
    // 未达阈值松手 = 干净单击，交还行内 onClick（浏览器原生触发），只清监听
    cleanup();
  };
  const onCancel = () => cleanup();

  /// **只认窗口自身失焦**。blur 不冒泡但会走捕获阶段，window 上的捕获监听
  /// 会收到页面内「任何元素」的 blur：呼出后焦点恒在搜索框（App.tsx 的
  /// zap://shown / 多处 inputRef.focus），用户第一次按结果行时 input 失焦
  /// 就会命中这里，把刚注册的 mousemove 监听清掉 → invoke 永不发出 →
  /// 「第一次拖动不生效、第二次才生效」（第二次焦点已不在搜索框）。
  /// 用 target 类型区分而非 `ev.target === window`：jsdom 与部分 WebView2
  /// 路径下 window 代理对象不严格等价。元素 / Document 的 blur 一律放行，
  /// 真·窗口失焦（Esc 收起 / 切到别的应用）仍清态防误起拖。
  const onBlur = (ev: FocusEvent) => {
    if (ev.target instanceof Element || ev.target instanceof Document) return;
    cleanup();
  };
  activeCleanup = cleanup;
  window.addEventListener("mousemove", onMove, true);
  window.addEventListener("mouseup", onUp, true);
  window.addEventListener("pointercancel", onCancel, true);
  window.addEventListener("blur", onBlur, true);
}
