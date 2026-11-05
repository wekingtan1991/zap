import { describe, expect, it } from "vitest";
import { routeItem } from "../dispatch";
import { kindUi } from "../registry";
import { initialPortOf } from "../ports/types";
import type { SearchItem } from "../types";

const portItem = (target: string): SearchItem => ({
  id: "port:q",
  title: `端口 ${target}`,
  subtitle: "",
  kind: "port",
  target,
  score: 0,
});

describe("端口工作台契约", () => {
  it("target 语义：all/entry = 全量，数字串 = 初始端口", () => {
    expect(initialPortOf("all")).toBeNull();
    expect(initialPortOf("entry")).toBeNull();
    expect(initialPortOf("8080")).toBe(8080);
    expect(initialPortOf("65535")).toBe(65535);
    for (const bad of [
      "",
      "0",
      "65536",
      "abc",
      "8080x",
      "-1",
      " 8080",
      "8080 ",
    ]) {
      expect(initialPortOf(bad), bad).toBeNull();
    }
  });

  it("port kind 走 activate 且徽标为端口", () => {
    expect(kindUi("port")?.label).toBe("端口");
    const route = routeItem(portItem("8080"));
    expect(route.type).toBe("activate");
    if (route.type === "activate") {
      expect(route.ui.activate).toBeTruthy();
    }
  });

  it("结果行无右键菜单：结束进程只能在工作台内鼠标显式确认", () => {
    expect(kindUi("port")?.ctxActions).toBeUndefined();
  });
});
