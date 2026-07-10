import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useStore } from "./store";
import HistoryModal from "./components/HistoryModal";
import SchemaTree from "./components/SchemaTree";
import TabBar from "./components/TabBar";
import QueryTab from "./components/QueryTab";
import ConnectionModal from "./components/ConnectionModal";
import DdlModal from "./components/DdlModal";
import ImportWizard from "./components/ImportWizard";
import ExportDialog from "./components/ExportDialog";
import DriverManager from "./components/DriverManager";
import StatusBar from "./components/StatusBar";

export default function App() {
  const init = useStore((s) => s.init);
  const tabs = useStore((s) => s.tabs);
  const activeTabId = useStore((s) => s.activeTabId);
  const connectionModal = useStore((s) => s.connectionModal);
  const ddlModal = useStore((s) => s.ddlModal);
  const importModal = useStore((s) => s.importModal);
  const exportModal = useStore((s) => s.exportModal);
  const driversModal = useStore((s) => s.driversModal);
  const historyModal = useStore((s) => s.historyModal);
  const setHistoryModal = useStore((s) => s.setHistoryModal);
  const toast = useStore((s) => s.toast);
  const addTab = useStore((s) => s.addTab);

  useEffect(() => {
    void init();
  }, [init]);

  // Native menubar "History → Show Query History…" (also ⌘Y).
  useEffect(() => {
    const un = listen("open-history", () => setHistoryModal(true));
    return () => {
      void un.then((f) => f());
    };
  }, [setHistoryModal]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "t") {
        e.preventDefault();
        addTab();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [addTab]);

  return (
    <div className="app">
      <aside className="sidebar">
        <SchemaTree />
      </aside>
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
        </div>
        <StatusBar />
      </main>

      {connectionModal.open && <ConnectionModal />}
      {ddlModal && <DdlModal target={ddlModal} />}
      {importModal && <ImportWizard target={importModal} />}
      {exportModal && <ExportDialog target={exportModal} />}
      {driversModal && <DriverManager />}
      {historyModal && <HistoryModal />}
      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}
