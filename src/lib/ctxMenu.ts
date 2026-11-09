/// 右键菜单定位：以光标为锚，下方放不下整体翻到光标上方、右侧放不下翻到
/// 光标左侧，翻转后仍越界再夹进视口。尺寸取菜单实测宽高与视口尺寸——不再
/// 按条目高度/面板常量估算（此前估算与实际渲染漂移，贴底时菜单末条被窗口
/// 下缘裁掉）。主列表与中转站共用。
import { useLayoutEffect, useState } from "react";

export const EDGE_GAP = 8;

export function placeMenuPos(
  x: number,
  y: number,
  width: number,
  height: number,
  viewW: number,
  viewH: number,
) {
  let left = x + width > viewW - EDGE_GAP ? x - width : x;
  left = Math.max(EDGE_GAP, Math.min(left, viewW - width - EDGE_GAP));
  let top = y + height > viewH - EDGE_GAP ? y - height : y;
  top = Math.max(EDGE_GAP, Math.min(top, viewH - height - EDGE_GAP));
  return { left, top };
}

/// 挂到菜单容器上的定位 hook：首帧绘制前完成实测+定位（useLayoutEffect），
/// 不会先闪一下错误位置。用回调 ref 而不是 useRef——菜单按条件挂载，同一
/// 坐标重开（条目数不同、尺寸变了）也要重新实测；x/y 为空表示菜单未开，
/// 返回 null 由调用方兜底。
export function useCtxMenuPos(x: number | null, y: number | null) {
  const [el, setEl] = useState<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    if (x === null || y === null || !el) {
      setPos(null);
      return;
    }
    setPos(
      placeMenuPos(
        x,
        y,
        el.offsetWidth,
        el.offsetHeight,
        window.innerWidth,
        window.innerHeight,
      ),
    );
  }, [x, y, el]);

  return { ref: setEl, el, pos };
}
