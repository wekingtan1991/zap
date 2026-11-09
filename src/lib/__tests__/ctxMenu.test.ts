import { describe, expect, it } from "vitest";
import { placeMenuPos } from "../ctxMenu";

/// 定位契约：以光标为左上锚点，下方放不下整体翻到光标上方、右侧放不下翻到
/// 光标左侧，翻转后仍越界再夹进视口（边距 8）。尺寸全部由调用方实测传入，
/// 不再依赖条目高度/面板常量估算——旧契约的常量漂移陷阱随实测方案消失
const VW = 720;
const VH = 480;
const W = 152;
const H = 130;

describe("placeMenuPos", () => {
  it("空间充足时以光标为左上角原位展开", () => {
    expect(placeMenuPos(100, 100, W, H, VW, VH)).toEqual({
      left: 100,
      top: 100,
    });
  });

  it("右侧放不下时翻到光标左侧", () => {
    const { left } = placeMenuPos(700, 100, W, H, VW, VH);
    expect(left).toBe(700 - W);
  });

  it("底部放不下时翻到光标上方（菜单底边不越视口）", () => {
    const { top } = placeMenuPos(100, 470, W, H, VW, VH);
    expect(top).toBe(470 - H);
    expect(top + H).toBeLessThanOrEqual(VH);
  });

  it("右下角同时放不下时双向翻转，翻转后仍越界再夹紧", () => {
    const { left, top } = placeMenuPos(710, 475, W, H, VW, VH);
    expect(left).toBe(710 - W);
    // 翻转后的底边仍越过边距 → 上提到视口下缘内
    expect(top).toBe(VH - H - 8);
    expect(top + H).toBeLessThanOrEqual(VH - 8);
  });

  it("菜单比可用视口还高时顶边优先夹紧，不出现负坐标", () => {
    const { top } = placeMenuPos(100, 470, W, 600, VW, VH);
    expect(top).toBe(8);
  });

  it("光标贴身左/上时也得留出边距", () => {
    const { left, top } = placeMenuPos(2, 2, W, H, VW, VH);
    expect(left).toBe(8);
    expect(top).toBe(8);
  });
});
