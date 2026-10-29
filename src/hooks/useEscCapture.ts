import { useEffect } from "react";

/// 窗口捕获兜 Esc：子页面点击空白后焦点掉 body（React 根之外），面板 onKeyDown
/// 收不到任何键，Esc 将失灵——全项目子页面通用（与 focusPanelRef 的挂载时接管互补）。
/// 不传依赖数组：每次渲染重挂，闭包永远拿到最新状态（与各视图自持状态的既有约定一致）。
/// enabled=false 时让位：同视图内更高优先级的 Esc 消费方（热键录制态 / 删除确认弹窗）
/// 由调用方用互斥的 enabled 表达，避免顺序耦合。
export function useEscCapture(onEsc: () => void, enabled = true) {
  useEffect(() => {
    if (!enabled) return;
    const onWinKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // IME 卫兵：合成期的 Esc 是取消选词，不是退出页面（全项目键盘面统一口径，
      // 与 App 合并面 / SuperPanel / StageView 的 isComposing 同款）
      if (e.isComposing) return;
      e.preventDefault();
      e.stopPropagation();
      onEsc();
    };
    window.addEventListener("keydown", onWinKey, true);
    return () => window.removeEventListener("keydown", onWinKey, true);
  });
}
