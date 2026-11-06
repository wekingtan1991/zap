/// 主面板常驻工具 Dock：结果列表与底栏之间的细条，空态 / 搜索态位置恒定。
/// chip 点击复用 App 的 runItem——stage 的 openStage、cmd 的 runCommand、
/// json/tool/port 的工作台 activate 全在那条分发链上，这里不另写激活逻辑。
/// 键盘面纪律：不新增任何组合键，chip 不进 Tab 序列（键盘路径由入口词承担）
import { useEffect, useRef, useState } from "react";
import {
  Braces,
  HardDrive,
  Layers,
  MoreHorizontal,
  Network,
  Type,
  Wrench,
  type LucideIcon,
} from "lucide-react";
import { badgeVariants } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { kindUi } from "@/plugins/registry";
import type { DockEntry, SearchItem } from "@/plugins/types";

/// 最多常驻 8 个 chip，其余收进 ⋯ 溢出菜单（纯鼠标，不改键盘面）
export const DOCK_VISIBLE_MAX = 8;

export function splitDockEntries(entries: DockEntry[], max = DOCK_VISIBLE_MAX) {
  return { visible: entries.slice(0, max), overflow: entries.slice(max) };
}

/// 悬停提示：入口词来自插件 PluginMeta.entries；前端状态注入的条目无入口词，
/// 退回条目自带说明（Everything 的一次性设置动作即此类）
export function dockTip(entry: DockEntry): string {
  if (entry.words.length === 0) return entry.item.subtitle || entry.item.title;
  return `入口词 · ${entry.words.slice(0, 4).join(" / ")}`;
}

/// 图标色不新造色值：整块复用 badge variant 的语义色（含浅色主题覆盖）
const DOCK_ICONS: Record<string, LucideIcon> = {
  json: Braces,
  tool: Type,
  port: Network,
  stage: Layers,
  cmd: HardDrive,
};

function ChipIcon({ item }: { item: SearchItem }) {
  const Icon = DOCK_ICONS[item.kind] ?? Wrench;
  return (
    <span
      className={cn(
        "zap-dock-ico",
        badgeVariants({ variant: kindUi(item.kind)?.badge ?? "secondary" })
      )}
    >
      <Icon aria-hidden="true" />
    </span>
  );
}

interface Props {
  entries: DockEntry[];
  onRun: (it: SearchItem) => void;
}

export function ToolDock({ entries, onRun }: Props) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const { visible, overflow } = splitDockEntries(entries);

  // 溢出菜单点外部即关。刻意不绑 keydown：Esc/方向键仍归主面板键盘面
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [open]);

  return (
    <div className="zap-dock" ref={rootRef}>
      <div className="zap-dock-chips">
        {visible.map((entry) => (
          <Tooltip key={entry.item.id}>
            <TooltipTrigger asChild>
              <button
                type="button"
                className="zap-dock-chip"
                tabIndex={-1}
                onClick={() => onRun(entry.item)}
              >
                <ChipIcon item={entry.item} />
                <span>{entry.label}</span>
              </button>
            </TooltipTrigger>
            <TooltipContent side="top">{dockTip(entry)}</TooltipContent>
          </Tooltip>
        ))}
        {overflow.length > 0 && (
          <div className="zap-dock-overflow">
            <button
              type="button"
              className="zap-dock-chip zap-dock-more"
              tabIndex={-1}
              aria-label={`更多工具（${overflow.length} 项）`}
              aria-expanded={open}
              onClick={() => setOpen((v) => !v)}
            >
              <MoreHorizontal aria-hidden="true" />
            </button>
            {open && (
              <div className="zap-dock-menu">
                {overflow.map((entry) => (
                  <button
                    key={entry.item.id}
                    type="button"
                    className="zap-dock-menu-item"
                    tabIndex={-1}
                    title={dockTip(entry)}
                    onClick={() => {
                      setOpen(false);
                      onRun(entry.item);
                    }}
                  >
                    <ChipIcon item={entry.item} />
                    <span>{entry.label}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
      <span className="zap-dock-note">常驻入口 · 不占结果行</span>
    </div>
  );
}
