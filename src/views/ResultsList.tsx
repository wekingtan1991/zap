/// 结果列表（自 App.tsx 拆出，审计 F4）：滚动容器 + 空态 + 行渲染。
/// 行 memo 化：hover 改 active 此前整列表重渲，行 props 全稳定后每次
/// hover 只重渲新旧两行（onHover/onRun/onContext/activeItemRef 均稳定引用，
/// actions 由父级 useMemo）。key 用条目 id（评审 P2-11）
import React, { type RefObject } from "react";
import { Lock, Pin, Search } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { kindUi, type ItemActions } from "@/plugins/registry";
import { isPinned, clipRowId, type SearchItem } from "@/plugins/types";

interface RowProps {
  it: SearchItem;
  index: number;
  isActive: boolean;
  /// 合并模式的非 clip 行禁用态（评审 P1-05）
  mergeDisabled: boolean;
  /// 合并模式下渲染行尾勾选框（仅文本剪贴板条目）
  showMergeCheck: boolean;
  /// 该行已被勾选（勾选框点亮）
  mergeChecked: boolean;
  activeItemRef: RefObject<HTMLDivElement | null>;
  onHover: (i: number) => void;
  onRun: (it: SearchItem) => void;
  onContext: (e: React.MouseEvent, it: SearchItem) => void;
  actions: ItemActions;
}

const ResultRow = React.memo(function ResultRow({
  it,
  index,
  isActive,
  mergeDisabled,
  showMergeCheck,
  mergeChecked,
  activeItemRef,
  onHover,
  onRun,
  onContext,
  actions,
}: RowProps) {
  const ui = kindUi(it.kind);
  return (
    <div
      ref={isActive ? activeItemRef : undefined}
      className={
        "zap-item" +
        (isActive ? " active" : "") +
        // 合并模式的非 clip 行禁用态（评审 P1-05）：点=选 心智下
        // 非目标行降透明度并吞点击，防误启动/误打开
        (mergeDisabled ? " merge-disabled" : "")
      }
      onMouseEnter={() => onHover(index)}
      onClick={() => onRun(it)}
      onContextMenu={(e) => onContext(e, it)}
    >
      {it.icon ? (
        <img className="zap-ico" src={it.icon} alt="" draggable={false} />
      ) : (
        <Badge variant={ui?.badge ?? "secondary"}>
          {ui?.label ?? it.kind}
        </Badge>
      )}
      <div className="zap-texts">
        <div className="zap-title">
          {isPinned(it) && <Pin className="zap-title-pin size-3" />}
          {it.title.replace(/^📌 /, "")}
        </div>
        {it.subtitle && <div className="zap-sub">{it.subtitle}</div>}
      </div>
      {/* 合并模式：文本剪贴板条目行尾勾选框，点选顺序即拼接顺序 */}
      {showMergeCheck && (
        <span
          className={"zap-merge-check" + (mergeChecked ? " on" : "")}
          aria-hidden="true"
        >
          ✓
        </span>
      )}
      {ui?.rowAction?.(it, actions)}
    </div>
  );
});

interface Props {
  items: SearchItem[];
  active: number;
  isVault: boolean;
  mergeMode: boolean;
  mergeSel: number[];
  /// 滚动容器 ref（父级 query 变化回滚顶部）与 active 行 ref（键盘导航
  /// 滚动补位）都归父级的 effect 消费，经 props 下挂
  resultsRef: RefObject<HTMLDivElement | null>;
  activeItemRef: RefObject<HTMLDivElement | null>;
  onHover: (i: number) => void;
  onRun: (it: SearchItem) => void;
  onContext: (e: React.MouseEvent, it: SearchItem) => void;
  actions: ItemActions;
}

export function ResultsList({
  items,
  active,
  isVault,
  mergeMode,
  mergeSel,
  resultsRef,
  activeItemRef,
  onHover,
  onRun,
  onContext,
  actions,
}: Props) {
  return (
    <div className="zap-results" ref={resultsRef}>
      {items.length === 0 ? (
        <div className="zap-empty">
          <span className="zap-empty-ico">
            {isVault ? (
              <Lock className="size-5" />
            ) : (
              <Search className="size-5" />
            )}
          </span>
          <span className="zap-empty-title">
            {isVault ? "没有匹配的账号" : "开始输入，即时搜索"}
          </span>
          <span className="zap-empty-sub">
            {isVault
              ? "换个关键词，或清空后按 Enter 新增账号"
              : "应用 · 文件 · 窗口 · 剪贴板 · 最近使用"}
          </span>
        </div>
      ) : (
        items.map((it, i) => (
          <ResultRow
            key={it.id}
            it={it}
            index={i}
            isActive={i === active}
            mergeDisabled={mergeMode && it.kind !== "clip"}
            showMergeCheck={mergeMode && it.kind === "clip"}
            mergeChecked={mergeSel.includes(clipRowId(it))}
            activeItemRef={activeItemRef}
            onHover={onHover}
            onRun={onRun}
            onContext={onContext}
            actions={actions}
          />
        ))
      )}
    </div>
  );
}
