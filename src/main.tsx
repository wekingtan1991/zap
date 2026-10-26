import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { SuperPanelRoot } from "./plugins/super/SuperPanel";
import "./index.css";

// 双窗口共用同一入口：superpanel 是超级面板的动作菜单（选中内容长按右键呼出），
// 主面板逻辑（搜索/呼出事件监听）不得在其中挂载
const isSuperPanel = getCurrentWindow().label === "superpanel";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isSuperPanel ? <SuperPanelRoot /> : <App />}
  </React.StrictMode>,
);
