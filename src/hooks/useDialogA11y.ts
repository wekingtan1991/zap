/// 自绘模态弹窗的焦点治理（2026-11-04 评审 R3 P1）：打开时聚焦弹窗内首个
/// 可聚焦元素、Tab/Shift+Tab 首尾循环困在弹窗内（给 aria-modal="true" 的
/// 承诺补上配套行为）、卸载时回焦打开者。不新增任何按键，Esc 归属不变
/// （仍由各弹窗既有监听处理）；窗口捕获挂载模式与 useEscCapture 同款。
import { useEffect, type RefObject } from "react";

const FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function useDialogA11y(
  ref: RefObject<HTMLElement | null>,
  active: boolean,
) {
  useEffect(() => {
    if (!active) return;
    const el = ref.current;
    if (!el) return;
    const opener =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const focusables = () =>
      Array.from(el.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
        (n) => n.offsetParent !== null,
      );
    // 初焦归弹窗：未显式 autoFocus 时落首元素（已 autoFocus 则同目标收敛）
    if (!(document.activeElement instanceof HTMLElement) || !el.contains(document.activeElement)) {
      focusables()[0]?.focus();
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      const list = focusables();
      if (list.length === 0) return;
      const first = list[0];
      const last = list[list.length - 1];
      const cur = document.activeElement;
      const inside = cur instanceof Node && el.contains(cur);
      if (e.shiftKey && (!inside || cur === first)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && (!inside || cur === last)) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      opener?.focus();
    };
  }, [active, ref]);
}
