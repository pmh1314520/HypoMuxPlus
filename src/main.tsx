import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { Hud } from "./components/Hud";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { SettingsProvider } from "./store";
import "./styles.css";

// 禁用浏览器默认右键菜单（桌面应用无需 WebView 原生菜单）
window.addEventListener("contextmenu", (e) => e.preventDefault());

// 拦截会导致页面重载 / 另存 / 打印等的浏览器默认快捷键，避免应用状态被意外重置
window.addEventListener(
  "keydown",
  (e) => {
    const k = e.key.toLowerCase();
    if (e.key === "F5") {
      e.preventDefault();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && ["s", "r", "p", "u", "f", "g", "j"].includes(k)) {
      // 放行输入框内的常规编辑，但这些组合在本应用中无浏览器语义，统一拦截
      e.preventDefault();
    }
  },
  true,
);

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
        <ErrorBoundary>
          <App />
        </ErrorBoundary>
      </SettingsProvider>
    </React.StrictMode>,
  );
}
