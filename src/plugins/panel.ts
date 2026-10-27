/// 插件子面板契约（二段式交互泛化）：插件专属面板与 App.tsx 之间只通过这三样耦合——
/// 1) Props 继承 PanelProps：onBack 由 App 装配（回主面板 + 恢复进入前搜索词 +
///    焦点回输入框），与 closeWorkbench 同一套退出语义
/// 2) 根节点 <div className="zap-panel outline-none" tabIndex={-1} ref={focusRef}>
/// 3) Esc 一律 useEscCapture(onBack, true) 窗口捕获——焦点掉 body 后面板 onKeyDown
///    收不到，这是栽过两次的坑（2026-10-22 表单 Esc / 2026-10-28 合并模式键盘面）
/// 新增面板 = App 的 View 加一个 kind + 渲染分支 + ItemActions 注册动作，
/// 不新增模式特判；首个消费者是图片预览面板（clipimg）
export interface PanelProps {
  onBack: () => void;
  /// 子面板焦点接管（防 Esc 失效）
  focusRef?: (el: HTMLDivElement | null) => void;
}
