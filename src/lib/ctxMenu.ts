/// 右键菜单定位：面板逻辑尺寸内夹紧，贴底/贴右时不溢出。
/// 主列表与中转站共用（此前两处各自夹法、常量漂移）。
/// 面板 720×480 与 lib.rs 的 PANEL_WIDTH/HEIGHT 及 tauri.conf.json 三处同步；
/// 菜单尺寸对应 ctx-menu.css：条目 ≈30px（7px×2 padding + 12.5px 行高）、
/// 容器 padding 4px×2 + 边框 2px，宽取 min-width 152px
const PANEL_W = 720;
const PANEL_H = 480;
const MENU_W = 152;
const ITEM_H = 30;
const MENU_CHROME = 10;
const EDGE_GAP = 8;

export function clampMenuPos(x: number, y: number, itemCount: number) {
  return {
    left: Math.min(x, PANEL_W - MENU_W - EDGE_GAP),
    top: Math.min(y, PANEL_H - (itemCount * ITEM_H + MENU_CHROME) - EDGE_GAP),
  };
}
