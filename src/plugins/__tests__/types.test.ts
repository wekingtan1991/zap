import { describe, expect, it } from "vitest";
import { clipRowId, isPinned, type SearchItem } from "../types";

const item = (over: Partial<SearchItem>): SearchItem => ({
  id: "clip:7",
  title: "t",
  subtitle: "",
  kind: "clip",
  target: "7",
  score: 0,
  ...over,
});

describe("types 契约", () => {
  it("clipRowId 从 target 提取行号——文本与图片两种 kind 行号寻址统一", () => {
    expect(clipRowId(item({}))).toBe(7);
    expect(clipRowId(item({ id: "clip:12345", target: "12345" }))).toBe(12345);
    // 图片条目同构：target = 行号，id 只是标识
    expect(
      clipRowId(item({ kind: "clipimg", id: "clipimg:42", target: "42" }))
    ).toBe(42);
    // 其余 kind 一律 NaN（防两套寻址键互相误用）
    expect(
      Number.isNaN(
        clipRowId(item({ id: "recent:1", kind: "recent", target: "x" }))
      )
    ).toBe(true);
    expect(Number.isNaN(clipRowId(item({ kind: "app", target: "1" })))).toBe(
      true
    );
    // 历史回归模式的反向锁：target 一旦退回内容本体（非数字），取数必须
    // 显式 NaN 而不是静默给出错误行号
    expect(
      Number.isNaN(clipRowId(item({ target: "剪贴板的正文内容" })))
    ).toBe(true);
  });

  it("isPinned 结构化字段优先，标题前缀兜底", () => {
    expect(isPinned(item({ pinned: true, title: "📌 x" }))).toBe(true);
    expect(isPinned(item({ pinned: false, title: "📌 x" }))).toBe(false);
    expect(isPinned(item({ title: "📌 旧后端" }))).toBe(true);
    expect(isPinned(item({ title: "普通" }))).toBe(false);
  });
});
