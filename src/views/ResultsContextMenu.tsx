/// 结果右键菜单展示层（自 App.tsx 拆出，审计 F2）：backdrop + 菜单 DOM +
/// 动作项渲染。ctxMenu 状态、键盘面窗口捕获 effect 与 11 分支动作链留在
/// AppView——模式化键盘面的监听生命周期与互斥顺序紧耦合（2026-10-22 以来
/// 三次事故同根因域），只切纯展示不切行为
import { clampMenuPos } from "@/lib/ctxMenu";
import { kindUi } from "@/plugins/registry";
import type { SearchItem } from "@/plugins/types";

interface Props {
  menu: { x: number; y: number; item: SearchItem };
  onClose: () => void;
  /// 动作分发在父级（stage_add/clip_pin/…/次级操作兜底），这里只上报动作名
  onAction: (act: string) => void;
}

export function ResultsContextMenu({ menu, onClose, onAction }: Props) {
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
        className="zap-ctx-menu"
        data-ctx-menu=""
        style={clampMenuPos(
          menu.x,
          menu.y,
          kindUi(menu.item.kind)?.ctxActions?.length ?? 0,
        )}
      >
        {(kindUi(menu.item.kind)?.ctxActions ?? []).map(([act, label]) => (
          <button
            key={act}
            type="button"
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
