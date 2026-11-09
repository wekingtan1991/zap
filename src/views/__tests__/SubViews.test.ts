import { describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import { renderSubViews, type SubViewProps } from "@/views/SubViews";
import type { View } from "@/views/appTypes";
import { EMPTY_FORM } from "@/plugins/vault/VaultFormView";

/// P3.5 第一步回归锁：子视图分支从 AppView 搬到 views/SubViews.tsx 后，
/// 逐个 kind 验证「分支没漏、组件没串、props 没接错」。
/// 只构造 ReactElement、不渲染：无需 mock Tauri、不碰焦点与键盘面。
const noop = () => {};
const base = {
  vault: {} as unknown as SubViewProps["vault"],
  updater: {} as unknown as SubViewProps["updater"],
  focusRef: noop,
  setForm: noop,
  closeImport: noop,
  closeSettings: noop,
  closeStage: noop,
  closeWorkbench: noop,
  repullClips: noop,
} satisfies Omit<SubViewProps, "view">;

const propsOf = (view: View, over: Partial<SubViewProps> = {}) => {
  const el = renderSubViews({ ...base, ...over, view });
  expect(el, `view.kind=${view.kind} 必须命中分支`).not.toBeNull();
  return el as ReactElement<Record<string, unknown>>;
};

describe("子视图分派", () => {
  it("main 返回 null（主面板由 AppView 自己渲染）", () => {
    expect(renderSubViews({ ...base, view: { kind: "main" } })).toBeNull();
  });

  it("10 个子视图 kind 各命中不同组件，无遗漏无重复", () => {
    const views: View[] = [
      { kind: "settings" },
      { kind: "stage" },
      { kind: "form", data: EMPTY_FORM },
      {
        kind: "detail",
        data: {
          id: 7,
          title: "标题",
          username: "u",
          password: "p",
          url: "",
          notes: "",
          pinned: 0,
          totp: "",
        },
      },
      { kind: "reveal", data: { id: 3, title: "标题", password: "p" } },
      { kind: "import" },
      { kind: "json", source: { kind: "text", text: "hello" }, backQuery: "bq" },
      { kind: "tool", source: { kind: "clip", rowid: 42 }, backQuery: "bq" },
      { kind: "port", port: 8080, backQuery: "bq" },
      { kind: "clipimg", rowid: 9, backQuery: "bq" },
    ];
    const types = new Set(views.map((v) => propsOf(v).type));
    expect(views).toHaveLength(10);
    expect(types.size, "每个 kind 必须落到各自的子组件").toBe(10);
  });

  it("分支 props 与 view 数据一一对应", () => {
    expect(propsOf({ kind: "form", data: EMPTY_FORM }).props.form).toBe(EMPTY_FORM);
    expect(
      propsOf({
        kind: "detail",
        data: {
          id: 7,
          title: "t",
          username: "u",
          password: "p",
          url: "",
          notes: "",
          pinned: 0,
          totp: "",
        },
        askDelete: true,
      }).props.initialConfirmDelete
    ).toBe(true);
    expect(
      propsOf({
        kind: "json",
        source: { kind: "text", text: "hello" },
        backQuery: "prev",
      }).props.text
    ).toBe("hello");
    expect(
      propsOf({
        kind: "tool",
        source: { kind: "clip", rowid: 42 },
        backQuery: "prev",
      }).props.clipRow
    ).toBe(42);
    expect(
      propsOf({ kind: "port", port: 8080, backQuery: "prev" }).props.initialPort
    ).toBe(8080);
    expect(
      propsOf({ kind: "clipimg", rowid: 9, backQuery: "prev" }).props.rowid
    ).toBe(9);
  });

  it("返回按钮把进入前的搜索词交还 closeWorkbench", () => {
    const closeWorkbench = vi.fn();
    const onBack = propsOf(
      { kind: "json", source: { kind: "empty" }, backQuery: "进入前的词" },
      { closeWorkbench }
    ).props.onBack as () => void;
    onBack();
    expect(closeWorkbench).toHaveBeenCalledWith("进入前的词");
  });
});
