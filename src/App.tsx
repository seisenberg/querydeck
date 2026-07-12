import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useStore } from "./store";
import HistoryModal from "./components/HistoryModal";
import ConnectionPicker from "./components/ConnectionPicker";
import TunnelsModal from "./components/TunnelsModal";
import SchemaTree from "./components/SchemaTree";
import TabBar, { dragRegionMouseDown } from "./components/TabBar";
import QueryTab from "./components/QueryTab";
import ConnectionModal from "./components/ConnectionModal";
import DdlModal from "./components/DdlModal";
import ImportWizard from "./components/ImportWizard";
import ExportDialog from "./components/ExportDialog";
import DriverManager from "./components/DriverManager";
import FormatterSettings from "./components/FormatterSettings";
import StatusBar from "./components/StatusBar";

export default function App() {
  const init = useStore((s) => s.init);
  const tabs = useStore((s) => s.tabs);
  const activeTabId = useStore((s) => s.activeTabId);
  const sidebarWidth = useStore((s) => s.sidebarWidth);
  const sidebarCollapsed = useStore((s) => s.sidebarCollapsed);
  const setSidebarWidth = useStore((s) => s.setSidebarWidth);
  const toggleSidebar = useStore((s) => s.toggleSidebar);
  const newConsoleRequested = useStore((s) => s.newConsoleRequested);
  const connectionModal = useStore((s) => s.connectionModal);
  const ddlModal = useStore((s) => s.ddlModal);
  const importModal = useStore((s) => s.importModal);
  const exportModal = useStore((s) => s.exportModal);
  const driversModal = useStore((s) => s.driversModal);
  const historyModal = useStore((s) => s.historyModal);
  const setHistoryModal = useStore((s) => s.setHistoryModal);
  const tunnelsModal = useStore((s) => s.tunnelsModal);
  const setTunnelsModal = useStore((s) => s.setTunnelsModal);
  const setDriversModal = useStore((s) => s.setDriversModal);
  const connectionPicker = useStore((s) => s.connectionPicker);
  const formatterModal = useStore((s) => s.formatterModal);
  const toast = useStore((s) => s.toast);

  useEffect(() => {
    void init();
  }, [init]);

  // Native menubar events: History → ⌘Y, File → SSH Tunnels… / Database Drivers…
  // ⌘W (File → Close Console) closes the active console; with none left it
  // closes the window itself. ⌘Q stays the stock Quit item.
  useEffect(() => {
    const subs = [
      listen("open-history", () => setHistoryModal(true)),
      listen("open-tunnels", () => setTunnelsModal(true)),
      listen("open-drivers", () => setDriversModal(true)),
      listen("close-console", () => {
        const s = useStore.getState();
        if (s.activeTabId) s.closeTab(s.activeTabId);
        else void getCurrentWindow().close();
      }),
      listen("reformat-query", () => useStore.getState().reformatActiveTab()),
      listen("open-formatter-settings", () =>
        useStore.getState().setFormatterModal(true),
      ),
    ];
    return () => {
      for (const s of subs) void s.then((f) => f());
    };
  }, [setHistoryModal, setTunnelsModal, setDriversModal]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "t") {
        e.preventDefault();
        newConsoleRequested();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [newConsoleRequested]);

  // Sidebar resize via pointer capture — survives re-renders mid-drag.
  const onResizerPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const onResizerPointerMove = (e: React.PointerEvent) => {
    if (e.buttons & 1) setSidebarWidth(e.clientX);
  };

  return (
    <div className={`app ${sidebarCollapsed ? "sidebar-is-collapsed" : ""}`}>
      {sidebarCollapsed ? (
        <aside className="sidebar sidebar-rail">
          {/* Empty strip: on macOS the traffic lights overlay this area, so
              the expand button lives BELOW it (a button here would sit
              underneath the close button — untappable and dangerous). */}
          <div className="rail-top" onMouseDown={dragRegionMouseDown} />
          <button
            className="small ghost rail-expand"
            title="Show connections"
            onClick={toggleSidebar}
          >
            »
          </button>
        </aside>
      ) : (
        <>
          <aside className="sidebar" style={{ width: sidebarWidth }}>
            <SchemaTree />
          </aside>
          <div
            className="v-resizer"
            onPointerDown={onResizerPointerDown}
            onPointerMove={onResizerPointerMove}
            title="Drag to resize"
          />
        </>
      )}
      <main className="main">
        <TabBar />
        <div className="tab-host">
          {tabs.map((t) => (
            <div
              key={t.id}
              className="tab-pane"
              style={{ display: t.id === activeTabId ? "flex" : "none" }}
            >
              <QueryTab tab={t} active={t.id === activeTabId} />
            </div>
          ))}
          {tabs.length === 0 && (
            <div className="placeholder" style={{ flex: 1, flexDirection: "column" }}>
              <div>No consoles open.</div>
              <button className="primary" onClick={() => newConsoleRequested()}>
                New console (⌘T)
              </button>
            </div>
          )}
        </div>
        <StatusBar />
      </main>

      {connectionModal.open && <ConnectionModal />}
      {ddlModal && <DdlModal target={ddlModal} />}
      {importModal && <ImportWizard target={importModal} />}
      {exportModal && <ExportDialog target={exportModal} />}
      {driversModal && <DriverManager />}
      {historyModal && <HistoryModal />}
      {tunnelsModal && <TunnelsModal />}
      {connectionPicker && <ConnectionPicker />}
      {formatterModal && <FormatterSettings />}
      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}
