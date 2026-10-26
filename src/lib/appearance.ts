/// 外观主题（深色/浅色/跟随系统）：documentElement 挂 zap-light 类切换令牌。
/// 深浅两套令牌定义在 index.css（:root 深色为默认，:root.zap-light 覆盖浅色）。
/// 设置页改模式时即时预览，持久化经 save_settings 的 appearance 键
export type Appearance = "dark" | "light" | "system";

const MEDIA = "(prefers-color-scheme: light)";
let mediaListener: (() => void) | null = null;

function effective(mode: Appearance): "dark" | "light" {
  if (mode === "system") {
    return window.matchMedia(MEDIA).matches ? "light" : "dark";
  }
  return mode;
}

/// 应用主题：挂/摘 documentElement 上的 zap-light 类；
/// system 模式注册媒体查询监听实时跟随系统切换，其余模式摘除
export function applyAppearance(mode: Appearance) {
  const root = document.documentElement;
  root.classList.toggle("zap-light", effective(mode) === "light");
  const mql = window.matchMedia(MEDIA);
  if (mode === "system") {
    if (!mediaListener) {
      mediaListener = () => applyAppearance("system");
      mql.addEventListener("change", mediaListener);
    }
  } else if (mediaListener) {
    mql.removeEventListener("change", mediaListener);
    mediaListener = null;
  }
}

/// 透明面板：documentElement 挂 zap-trans 类，CSS 侧把面板/弹层表面切半透明
/// （窗口特效=后端亚克力，本类只管网页表面；特效失败时后端不落偏好，
/// 前端以偏好为准挂类——特效失败的场景面板表面仍半透明但无模糊底，可读性靠 alpha 下限保底）
export function applyPanelTransparency(on: boolean) {
  document.documentElement.classList.toggle("zap-trans", on);
}
