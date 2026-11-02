import { describe, expect, it } from "vitest";
import {
  clampOffset,
  clampScale,
  fitZoomState,
  zoomAtPoint,
  zoomCentered,
  ZOOM_MAX,
  ZOOM_MIN,
} from "@/plugins/clipimg/zoom";

/// 图片预览缩放数学（clipimg/zoom.ts）：需求口径「最小只能是完全展示」
/// 即倍率下限 = fit（1 倍），此时恒居中、不可平移
describe("图片预览缩放数学", () => {
  it("倍率夹紧在 [1, 8]", () => {
    expect(clampScale(0.5)).toBe(ZOOM_MIN);
    expect(clampScale(1)).toBe(1);
    expect(clampScale(100)).toBe(ZOOM_MAX);
    expect(clampScale(3)).toBe(3);
  });

  it("平移夹紧：某轴不足视口居中，超出后端点不越过视口边", () => {
    // 200×100 图、100×100 视口、2 倍 → 400×200：x 可移 ±150，y 可移 ±50
    expect(
      clampOffset(0, 0, 2, { w: 100, h: 100 }, { w: 200, h: 100 }),
    ).toEqual({
      x: 0,
      y: 0,
    });
    expect(
      clampOffset(999, -999, 2, { w: 100, h: 100 }, { w: 200, h: 100 }),
    ).toEqual({ x: 150, y: -50 });
  });

  it("下限=完全展示：fit 态任何方向缩放都原地不动", () => {
    const fit = fitZoomState();
    expect(
      zoomAtPoint(
        fit,
        { x: 40, y: 40 },
        0.5,
        { w: 100, h: 100 },
        { w: 200, h: 100 },
      ),
    ).toEqual(fit);
    expect(
      zoomCentered(fit, 0.8, { w: 100, h: 100 }, { w: 200, h: 100 }),
    ).toEqual(fit);
  });

  it("指点缩放：光标下的图像点缩放前后不动（图像局部坐标不变）", () => {
    const prev = { s: 2, ox: 30, oy: -10 };
    const viewport = { w: 300, h: 200 };
    const fit = { w: 300, h: 150 };
    // 光标相对视口中心
    const m = { x: -110, y: -80 };
    const next = zoomAtPoint(prev, m, 1.5, viewport, fit);
    expect(next.s).toBe(3);
    // q = (m − o)/s 为光标处图像局部坐标，前后恒等
    expect((m.x - next.ox) / next.s).toBeCloseTo((m.x - prev.ox) / prev.s);
    expect((m.y - next.oy) / next.s).toBeCloseTo((m.y - prev.oy) / prev.s);
  });

  it("指点缩放到上限：倍率停在上限、锚点等式仍成立", () => {
    const prev = { s: 7, ox: 0, oy: 0 };
    const viewport = { w: 300, h: 200 };
    const fit = { w: 300, h: 150 };
    const m = { x: 50, y: -20 };
    const next = zoomAtPoint(prev, m, 2, viewport, fit);
    expect(next.s).toBe(ZOOM_MAX);
    expect((m.x - next.ox) / next.s).toBeCloseTo((m.x - prev.ox) / prev.s);
    expect((m.y - next.oy) / next.s).toBeCloseTo((m.y - prev.oy) / prev.s);
  });

  it("中心缩放：平移按倍率等比", () => {
    const prev = { s: 2, ox: 40, oy: 20 };
    const next = zoomCentered(prev, 2, { w: 100, h: 80 }, { w: 90, h: 60 });
    expect(next.s).toBe(4);
    expect(next).toEqual({ s: 4, ox: 80, oy: 40 });
  });

  it("到达上限后继续放大原地不动（同引用）", () => {
    const maxed = { s: ZOOM_MAX, ox: 0, oy: 0 };
    expect(zoomCentered(maxed, 2, { w: 100, h: 100 }, { w: 200, h: 100 })).toBe(
      maxed,
    );
    expect(
      zoomAtPoint(
        maxed,
        { x: 10, y: 10 },
        2,
        { w: 100, h: 100 },
        { w: 200, h: 100 },
      ),
    ).toBe(maxed);
  });

  it("缩回下限时平移归零（重新完全展示恒居中；fit 恒 ≤ 视口）", () => {
    const prev = { s: 2, ox: 40, oy: -30 };
    const next = zoomCentered(
      prev,
      0.25,
      { w: 200, h: 100 },
      { w: 150, h: 80 },
    );
    expect(next.s).toBe(ZOOM_MIN);
    expect(next.ox).toBe(0);
    expect(next.oy).toBe(0);
  });
});
