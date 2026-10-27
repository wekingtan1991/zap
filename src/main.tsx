import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { SuperPanelRoot } from "./plugins/super/SuperPanel";
import { StageDropRoot } from "./plugins/stage/StageDropRoot";
import "./styles/index.css";

// 三窗口共用同一入口：superpanel 是超级面板的动作菜单，stagedrop 是中转站
// 的拖拽落区条，主面板逻辑（搜索/呼出事件监听）不得在其中挂载
const label = getCurrentWindow().label;

// 落区条文档透明化：base.css 的全局 body 底色会把窗口透明边距与卡片圆角
// 外露处填成实色（三窗口共用同一文档），挂类豁免（样式见 stage.css）
if (label === "stagedrop") {
  document.documentElement.classList.add("zap-stagedrop-doc");
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {label === "superpanel" ? (
      <SuperPanelRoot />
    ) : label === "stagedrop" ? (
      <StageDropRoot />
    ) : (
      <App />
    )}
  </React.StrictMode>,
);
