import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { isThemePref } from "./theme";

// 便签窗口：URL 存在 `note` 参数即走便签壳；动态导入令 note.css 只在便签窗口分包加载。
const params = new URLSearchParams(location.search);
const root = document.getElementById("root") as HTMLElement;

if (params.has("note")) {
  const parsed = Number.parseInt(params.get("note") ?? "", 10);
  const colorIndex = Number.isNaN(parsed) ? 0 : parsed;
  const themeParam = params.get("theme");
  const themePref = isThemePref(themeParam) ? themeParam : "system";

  void import("./NoteApp").then(({ default: NoteApp }) => {
    ReactDOM.createRoot(root).render(
      <React.StrictMode>
        <NoteApp colorIndex={colorIndex} themePref={themePref} />
      </React.StrictMode>,
    );
  });
} else {
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}
