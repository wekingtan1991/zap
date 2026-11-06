import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { DockEntry, SearchItem } from "@/plugins/types";
import {
  DOCK_VISIBLE_MAX,
  ToolDock,
  dockTip,
  splitDockEntries,
} from "../ToolDock";

const makeEntry = (n: number, words: string[] = []): DockEntry => ({
  item: {
    id: `tool:${n}`,
    title: `工具 ${n}`,
    subtitle: `说明 ${n}`,
    kind: "tool",
    target: "entry",
    score: 0,
  } satisfies SearchItem,
  label: `工具 ${n}`,
  words,
});

describe("splitDockEntries（超过 8 个收进 ⋯）", () => {
  it("不超过 8 个全部常驻，无溢出", () => {
    const entries = Array.from({ length: 8 }, (_, i) => makeEntry(i));
    const { visible, overflow } = splitDockEntries(entries);
    expect(visible).toHaveLength(DOCK_VISIBLE_MAX);
    expect(overflow).toHaveLength(0);
  });

  it("第 9 个起进溢出菜单，顺序不变", () => {
    const entries = Array.from({ length: 10 }, (_, i) => makeEntry(i));
    const { visible, overflow } = splitDockEntries(entries);
    expect(visible.map((e) => e.label)).toEqual(
      entries.slice(0, 8).map((e) => e.label)
    );
    expect(overflow.map((e) => e.label)).toEqual(["工具 8", "工具 9"]);
  });
});

describe("dockTip（入口词来自插件声明）", () => {
  it("有入口词时展示，最多四个", () => {
    const tip = dockTip(makeEntry(1, ["json", "校验", "yaml", "美化", "压缩"]));
    expect(tip).toContain("json");
    expect(tip).toContain("美化");
    expect(tip).not.toContain("压缩");
  });

  it("无入口词的状态注入条目退回自带说明", () => {
    expect(dockTip(makeEntry(1))).toBe("说明 1");
  });
});

describe("ToolDock 渲染", () => {
  it("常驻细条 + 零新增键盘面：chip 全不进 Tab 序列，溢出入口出现", () => {
    const entries = Array.from({ length: 9 }, (_, i) => makeEntry(i));
    // 真实 App 的 TooltipProvider 挂在最外层（App.tsx），单测需自带
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <ToolDock entries={entries} onRun={() => {}} />
      </TooltipProvider>
    );
    expect(html).toContain("zap-dock");
    expect(html).toContain("更多工具（1 项）");
    expect(html).not.toContain('tabindex="0"');
    expect(html.match(/tabindex="-1"/g)).toHaveLength(DOCK_VISIBLE_MAX + 1);
  });
});
