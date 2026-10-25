import { describe, expect, it } from "vitest";
import { routeItem } from "../dispatch";
import type { SearchItem } from "../types";

const item = (over: Partial<SearchItem>): SearchItem => ({
  id: "x:1",
  title: "t",
  subtitle: "",
  kind: "app",
  target: "",
  score: 0,
  ...over,
});

describe("routeItem（runItem 的分发决策）", () => {
  it("保护域伪条目走表单/导入", () => {
    expect(routeItem(item({ id: "vault:add", kind: "info" })).type).toBe(
      "form-add"
    );
    expect(routeItem(item({ id: "vault:import", kind: "info" })).type).toBe(
      "import"
    );
  });

  it("有专属激活行为的 kind 走 activate 且携带注册表 UI", () => {
    for (const kind of ["vault", "json", "tool", "cmd"]) {
      const route = routeItem(item({ kind }));
      expect(route.type, kind).toBe("activate");
      if (route.type === "activate") {
        expect(route.ui.activate, kind).toBeTruthy();
      }
    }
  });

  it("外发 kind 落 openExternal", () => {
    for (const kind of ["app", "clip", "file", "win", "recent", "settings", "web", "url"]) {
      const route = routeItem(item({ kind }));
      expect(route.type, kind).toBe("external");
    }
  });

  it("纯展示 kind（info）不外发", () => {
    expect(routeItem(item({ kind: "info" })).type).toBe("none");
  });

  it("未注册 kind 按旧约定交后端（external）", () => {
    expect(routeItem(item({ kind: "ghost-kind" })).type).toBe("external");
  });
});
