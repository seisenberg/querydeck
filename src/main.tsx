import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

/** Platform class for native-feel styling: font stacks, scrollbar treatment,
 * control radii and the macOS overlay-titlebar inset all key off this. */
const ua = navigator.userAgent;
document.documentElement.dataset.platform = ua.includes("Mac")
  ? "mac"
  : ua.includes("Windows")
    ? "windows"
    : "linux";

/** Last-resort crash surface: a render error must never leave a silent blank
 * window — show what broke so it can be reported and the app restarted. */
class CrashBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("Render crash:", error, info.componentStack);
  }
  render() {
    if (this.state.error) {
      return (
        <div style={{ padding: 24, fontFamily: "ui-monospace, monospace", color: "#f66" }}>
          <h2 style={{ color: "#f88" }}>QueryDeck hit a rendering error</h2>
          <pre style={{ whiteSpace: "pre-wrap", fontSize: 12 }}>
            {String(this.state.error?.stack ?? this.state.error)}
          </pre>
          <button onClick={() => window.location.reload()} style={{ marginTop: 12 }}>
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

window.addEventListener("error", (e) => {
  const el = document.getElementById("crash-banner") ?? document.createElement("div");
  el.id = "crash-banner";
  el.style.cssText =
    "position:fixed;bottom:0;left:0;right:0;z-index:99999;background:#400;color:#fbb;font:12px ui-monospace;padding:8px;white-space:pre-wrap";
  el.textContent = `Uncaught: ${e.message} @ ${e.filename}:${e.lineno}`;
  document.body.appendChild(el);
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <CrashBoundary>
      <App />
    </CrashBoundary>
  </React.StrictMode>,
);
