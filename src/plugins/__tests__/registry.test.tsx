import { describe, expect, it } from "vitest";
import { kindUi } from "../registry";

/// 注册表契约锁定：每个 kind 的徽标文案与行为标志。
/// 改动这里之前先想清楚 runItem 分发与列表渲染的既有预期
describe("kindUi 注册表", () => {
  it("每个 kind 都有中文徽标", () => {
    const expectations: Record<string, string> = {
      app: "应用",
      clip: "剪贴板",
      clipimg: "图片",
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
    const withActivate = ["vault", "json", "tool", "cmd", "stage"];
    const withoutActivate = [
      "app",
      "clip",
      "clipimg",
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

  it("右键动作按 kind 收敛：路径类/剪贴板/窗口/链接各有其菜单", () => {
    // 应用四动作（管理员运行仅 app；放入中转站排首位，同 file/recent 口径）
    expect(kindUi("app")?.ctxActions).toEqual([
      ["stage_add", "放入中转站"],
      ["reveal", "打开所在文件夹"],
      ["admin", "管理员运行"],
      ["copy_path", "复制路径"],
    ]);
    // recent 是 lnk 指针，复用路径类动作（后端解析真实目标）；
    // 放入中转站排首位（主路径动作，.lnk 由后端 stage_add 解析为真实目标）
    expect(kindUi("recent")?.ctxActions).toEqual([
      ["stage_add", "放入中转站"],
      ["reveal", "打开所在文件夹"],
      ["copy_path", "复制路径"],
    ]);
    expect(kindUi("file")?.ctxActions).toEqual(kindUi("recent")?.ctxActions);
    // 剪贴板：仅复制（不粘贴）+ 固定；窗口：关闭
    expect(kindUi("clip")?.ctxActions).toEqual([
      ["clip_copy", "仅复制（不粘贴）"],
      ["clip_pin", "固定 / 取消固定"],
    ]);
    // 剪贴板图片：预览大图排首位（右键专属入口，Enter 保留回贴）+ 固定
    expect(kindUi("clipimg")?.ctxActions).toEqual([
      ["clipimg_preview", "预览大图"],
      ["clip_pin", "固定 / 取消固定"],
    ]);
    expect(kindUi("win")?.ctxActions).toEqual([["close_window", "关闭窗口"]]);
    // 网页/捷径结果（kind=web）与网址：仅复制链接
    expect(kindUi("web")?.ctxActions).toEqual([["copy_path", "复制链接"]]);
    expect(kindUi("url")?.ctxActions).toEqual([["copy_path", "复制链接"]]);
    // 工具/状态/功能/设置/账号无右键菜单（undefined = 只压默认菜单）
    for (const kind of ["json", "tool", "cmd", "info", "settings", "vault"]) {
      expect(kindUi(kind)?.ctxActions, kind).toBeUndefined();
    }
  });
});
