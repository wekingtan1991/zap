import { describe, expect, it } from "vitest";
import { clampMenuPos } from "../ctxMenu";

/// clampMenuPos 的常量（PANEL_W/H、MENU_W、ITEM_H、MENU_CHROME、EDGE_GAP）
/// 与三处手工同步点（ctx-menu.css 的条目高度与容器 chrome、lib.rs 的
/// PANEL_WIDTH/HEIGHT、tauri.conf.json）构成漂移陷阱——测试按「菜单必须
/// 完整落在面板内」的行为契约锁定，常量改了这里会红
describe("clampMenuPos", () => {
  it("面板内坐标原样返回", () => {
    const { left, top } = clampMenuPos(100, 100, 3);
    expect(left).toBe(100);
    expect(top).toBe(100);
  });

  it("贴右溢出时左移到面板右缘内（含边距）", () => {
    const { left } = clampMenuPos(700, 100, 3);
    expect(left).toBeLessThanOrEqual(720 - 152 - 8);
  });

  it("贴底的多条目菜单整体上移不裁切（最高条数场景）", () => {
    const { top } = clampMenuPos(100, 470, 5);
    // 菜单总高 5*30+10=160，底缘不得越过 480-8
    expect(top).toBeLessThanOrEqual(480 - (5 * 30 + 10) - 8);
  });

  it("右下角同时溢出时双向都夹紧", () => {
    const { left, top } = clampMenuPos(719, 479, 4);
    expect(left).toBeLessThanOrEqual(720 - 152 - 8);
    expect(top).toBeLessThanOrEqual(480 - (4 * 30 + 10) - 8);
  });

  it("超大条目数仍不出面板（长菜单贴底场景）", () => {
    const { left, top } = clampMenuPos(400, 400, 12);
    expect(left).toBe(400);
    expect(top).toBeLessThanOrEqual(480 - (12 * 30 + 10) - 8);
  });
});
