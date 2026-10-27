import { describe, expect, it } from "vitest";
import { clipImageRowId, clipRowId, isPinned, type SearchItem } from "../types";

const item = (over: Partial<SearchItem>): SearchItem => ({
  id: "clip:7",
  title: "t",
  subtitle: "",
  kind: "clip",
  target: "剪贴板的正文内容，不是行号",
  score: 0,
  ...over,
});

describe("types 契约", () => {
  it("clipRowId 从 id 提取行号——target 是内容本体不是行号", () => {
    // 回归锁定：合并粘贴首版按 target 取行号恒 NaN，勾选整条路径静默失灵
    expect(clipRowId(item({}))).toBe(7);
    expect(clipRowId(item({ id: "clip:12345" }))).toBe(12345);
    // 非 clip 条目给 NaN
    expect(Number.isNaN(clipRowId(item({ id: "recent:1", kind: "recent" })))).toBe(
      true
    );
  });

  it("clipImageRowId 与 clip 相反：行号在 target，两种 kind 寻址键不互套", () => {
    // 图片条目后端按 target 寻址（open_item/预览同源），id 只是标识
    expect(clipImageRowId(item({ kind: "clipimg", target: "42" }))).toBe(42);
    expect(
      clipImageRowId(item({ kind: "clipimg", id: "clipimg:999", target: "42" }))
    ).toBe(42);
    // 文本 clip 与其余 kind 一律 NaN（防两套寻址键互相误用）
    expect(Number.isNaN(clipImageRowId(item({})))).toBe(true);
    expect(Number.isNaN(clipImageRowId(item({ kind: "app" })))).toBe(true);
  });

  it("isPinned 结构化字段优先，标题前缀兜底", () => {
    expect(isPinned(item({ pinned: true, title: "📌 x" }))).toBe(true);
    expect(isPinned(item({ pinned: false, title: "📌 x" }))).toBe(false);
    expect(isPinned(item({ title: "📌 旧后端" }))).toBe(true);
    expect(isPinned(item({ title: "普通" }))).toBe(false);
  });
});
