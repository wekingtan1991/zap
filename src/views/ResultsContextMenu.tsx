/// 结果右键菜单展示层（自 App.tsx 拆出，审计 F2）：backdrop + 菜单 DOM +
/// 动作项渲染。ctxMenu 状态、键盘面窗口捕获 effect 与 11 分支动作链留在
/// AppView——模式化键盘面的监听生命周期与互斥顺序紧耦合（2026-10-22 以来
/// 三次事故同根因域），只切纯展示不切行为。
/// 可达性（2026-11-04 评审 R3 P1）：menu/menuitem 语义 + 打开即聚焦首项 +
/// 菜单内方向键循环——此前菜单对键盘与 SR 完全不可达（真实 button 永不被
/// 聚焦，Enter 通路形同虚设）。方向键是键盘面既定许可键，不新增组合键；
/// Esc 关闭与 Enter 放行仍由 AppView 的窗口捕获先行处理（注册顺序在前）
import { useEffect } from "react";
import { useCtxMenuPos } from "@/lib/ctxMenu";
import { kindUi } from "@/plugins/registry";
import type { SearchItem } from "@/plugins/types";

interface Props {
  menu: { x: number; y: number; item: SearchItem };
  onClose: () => void;
  /// 动作分发在父级（stage_add/clip_pin/…/次级操作兜底），这里只上报动作名
  onAction: (act: string) => void;
}

export function ResultsContextMenu({ menu, onClose, onAction }: Props) {
  const { ref: menuRef, el, pos } = useCtxMenuPos(menu.x, menu.y);

  useEffect(() => {
    if (!el) return;
    // 打开即聚焦首项：键盘用户落在菜单内（Tab 原生在按钮间循环，Enter
    // 原生 click 由 AppView 的 inMenu 放行直接可用）
    el.querySelector<HTMLElement>(".zap-ctx-item")?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing) return;
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      const items = Array.from(
        el.querySelectorAll<HTMLElement>(".zap-ctx-item"),
      );
      if (items.length === 0) return;
      e.preventDefault();
      e.stopPropagation();
      const cur = items.indexOf(
        document.activeElement as HTMLElement,
      );
      // 未聚焦/首尾越界都回到循环里的下一站（与主面板列表循环同口径）
      const next =
        e.key === "ArrowDown"
          ? (cur + 1 + items.length) % items.length
          : (cur - 1 + items.length) % items.length;
      items[next].focus();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [el]);

  return (
    <>
      <div
        className="zap-ctx-backdrop"
        onClick={onClose}
        onContextMenu={(e) => {
          e.preventDefault();
          onClose();
        }}
      />
      <div
        ref={menuRef}
        className="zap-ctx-menu"
        data-ctx-menu=""
        role="menu"
        aria-label={`「${menu.item.title.replace(/^📌 /, "")}」的操作`}
        style={pos ?? { left: menu.x, top: menu.y }}
      >
        {(kindUi(menu.item.kind)?.ctxActions ?? []).map(([act, label]) => (
          <button
            key={act}
            type="button"
            role="menuitem"
            className="zap-ctx-item"
            onClick={() => {
              onClose();
              onAction(act);
            }}
          >
            {label}
          </button>
        ))}
      </div>
    </>
  );
}
