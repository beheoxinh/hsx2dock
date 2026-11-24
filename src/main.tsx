import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import "./index.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary fullScreen>
      <App />
    </ErrorBoundary>
  </React.StrictMode>
);

// The static boot screen in index.html covers the window between the webview
// opening and React's first paint. Two frames after render guarantee the shell
// is already painted underneath, so the fade reveals the app instead of a
// flash of empty background.
const splash = document.getElementById("app-splash");
if (splash) {
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      splash.classList.add("is-leaving");
      window.setTimeout(() => splash.remove(), 400);
    });
  });
}
