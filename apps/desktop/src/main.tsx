import React from "react";
import ReactDOM from "react-dom/client";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import Overlay from "./Overlay";
import { resolveWindowLabel } from "./lib/window-routing";
import "./styles.css";

const development = (import.meta as ImportMeta & { env: { DEV: boolean } }).env
  .DEV;
const previewWindow = development
  ? new URLSearchParams(window.location.search).get("window")
  : null;
const windowLabel = resolveWindowLabel({
  development,
  previewWindow,
  currentWindowLabel: isTauri() ? getCurrentWindow().label : null,
});
document.documentElement.dataset.window = windowLabel;

// Модалка должна раскрываться из той точки, где её позвали: запоминаем место
// последнего нажатия, `.confirm-dialog` берёт его как transform-origin.
// ponytail: одна пара переменных на документ вместо проброса координат через
// React — точка нажатия всегда одна, второй модалки одновременно не бывает.
if (windowLabel !== "overlay") {
  window.addEventListener(
    "pointerdown",
    (event) => {
      const style = document.documentElement.style;
      style.setProperty("--origin-x", `${event.clientX}px`);
      style.setProperty("--origin-y", `${event.clientY}px`);
    },
    { passive: true },
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {windowLabel === "overlay" ? <Overlay /> : <App />}
  </React.StrictMode>,
);
