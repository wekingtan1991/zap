/// 图片预览缩放纯数学：倍率以「完全展示（fit，即容器内 max 约束贴合）」为
/// 1 倍基准——下限即完全展示（需求口径：最小只能完全展示图片），上限 8 倍。
/// 组件只负责测量（视口尺寸 / fit 尺寸）与渲染，状态迁移全部走这里，可单测。

/// 倍率下限 = 完全展示；上限给够截图读小字（fit 常已小于原生尺寸）
export const ZOOM_MIN = 1;
export const ZOOM_MAX = 8;
/// 工具条 ± 与双击回适应之外的档位系数（每击 ×1.25）
export const ZOOM_STEP = 1.25;
/// 滚轮每档系数
export const ZOOM_WHEEL = 1.2;

export interface Point {
  x: number;
  y: number;
}

export interface Size {
  w: number;
  h: number;
}

/// 缩放状态：s 为相对 fit 的倍率；ox/oy 为图像中心相对视口中心的平移
/// （屏幕像素）。s = MIN 时恒居中（ox/oy = 0）
export interface ZoomState {
  s: number;
  ox: number;
  oy: number;
}

/// 完全展示态：1 倍居中
export const fitZoomState = (): ZoomState => ({ s: ZOOM_MIN, ox: 0, oy: 0 });

const clamp = (v: number, lo: number, hi: number) =>
  Math.min(hi, Math.max(lo, v));

export const clampScale = (s: number): number => clamp(s, ZOOM_MIN, ZOOM_MAX);

/// 平移夹紧（no-gap）：某轴缩放后不足视口 → 该轴居中（0）；
/// 超出视口 → 端点不得越过视口边缘（可移幅度 = (缩放尺寸 − 视口) / 2）
export const clampOffset = (
  ox: number,
  oy: number,
  s: number,
  viewport: Size,
  fit: Size,
): Point => {
  const hx = (fit.w * s - viewport.w) / 2;
  const hy = (fit.h * s - viewport.h) / 2;
  return {
    x: hx <= 0 ? 0 : clamp(ox, -hx, hx),
    y: hy <= 0 ? 0 : clamp(oy, -hy, hy),
  };
};

/// 指点缩放（滚轮）：光标下的图像点缩放前后不动。
/// point 为光标相对视口中心的坐标；推导：q = (p − C)/s 不变 ⇒
/// o′ = m − k·(m − o)，k = s′/s，m = 光标相对视口中心
export const zoomAtPoint = (
  prev: ZoomState,
  point: Point,
  factor: number,
  viewport: Size,
  fit: Size,
): ZoomState => {
  const s = clampScale(prev.s * factor);
  if (s === prev.s) return prev;
  const k = s / prev.s;
  const o = clampOffset(
    point.x - k * (point.x - prev.ox),
    point.y - k * (point.y - prev.oy),
    s,
    viewport,
    fit,
  );
  return { s, ox: o.x, oy: o.y };
};

/// 中心缩放（工具条 ±）：视口中心为锚（point = 0），平移向量按倍率等比
export const zoomCentered = (
  prev: ZoomState,
  factor: number,
  viewport: Size,
  fit: Size,
): ZoomState => {
  const s = clampScale(prev.s * factor);
  if (s === prev.s) return prev;
  const k = s / prev.s;
  const o = clampOffset(prev.ox * k, prev.oy * k, s, viewport, fit);
  return { s, ox: o.x, oy: o.y };
};
