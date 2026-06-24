import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { Hud } from "./components/Hud";
import { SettingsProvider } from "./store";
import "./styles.css";

// 禁用浏览器默认右键菜单（桌面应用无需 WebView 原生菜单）
window.addEventListener("contextmenu", (e) => e.preventDefault());

const isHud = new URLSearchParams(window.location.search).get("hud") === "1";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

if (isHud) {
  // 悬浮窗为独立轻量应用，不挂载主界面的 Provider 树
  root.render(
    <React.StrictMode>
      <Hud />
    </React.StrictMode>,
  );
} else {
  root.render(
    <React.StrictMode>
      <SettingsProvider>
        <App />
      </SettingsProvider>
    </React.StrictMode>,
  );
}
