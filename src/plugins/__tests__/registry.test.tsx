import { describe, expect, it } from "vitest";
import { kindUi } from "../registry";

/// 注册表契约锁定：每个 kind 的徽标文案与行为标志。
/// 改动这里之前先想清楚 runItem 分发与列表渲染的既有预期
describe("kindUi 注册表", () => {
  it("每个 kind 都有中文徽标", () => {
    const expectations: Record<string, string> = {
      app: "应用",
      clip: "剪贴板",
      file: "文件",
      win: "窗口",
      recent: "最近",
      settings: "设置",
      vault: "账号",
      web: "搜索",
      url: "网址",
      json: "JSON",
      tool: "工具",
      cmd: "功能",
      info: "状态",
    };
    for (const [kind, label] of Object.entries(expectations)) {
      expect(kindUi(kind)?.label, kind).toBe(label);
    }
  });

  it("专属激活行为只出现在预期的 kind 上", () => {
    const withActivate = ["vault", "json", "tool", "cmd"];
    const withoutActivate = [
      "app",
      "clip",
      "file",
      "win",
      "recent",
      "settings",
      "web",
      "url",
    ];
    for (const kind of withActivate) {
      expect(kindUi(kind)?.activate, kind).toBeTruthy();
    }
    for (const kind of withoutActivate) {
      expect(kindUi(kind)?.activate, kind).toBeUndefined();
      // 未显式关闭外发 = 落 openExternal
      expect(kindUi(kind)?.external, kind).not.toBe(false);
    }
  });

  it("info 为纯展示条目（关闭外发）", () => {
    expect(kindUi("info")?.external).toBe(false);
  });

  it("未注册 kind 返回 undefined（渲染退回原文案）", () => {
    expect(kindUi("nope")).toBeUndefined();
  });

  it("vault 与 clip 配有行尾动作（置顶/固定按钮）", () => {
    expect(kindUi("vault")?.rowAction).toBeTruthy();
    expect(kindUi("clip")?.rowAction).toBeTruthy();
    expect(kindUi("app")?.rowAction).toBeUndefined();
  });
});
