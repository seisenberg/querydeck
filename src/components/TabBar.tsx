import { useStore } from "../store";

export default function TabBar() {
  const tabs = useStore((s) => s.tabs);
  const activeTabId = useStore((s) => s.activeTabId);
  const setActiveTab = useStore((s) => s.setActiveTab);
  const closeTab = useStore((s) => s.closeTab);
  const setConnectionPicker = useStore((s) => s.setConnectionPicker);
  const profiles = useStore((s) => s.profiles);

  return (
    <div className="tabbar">
      {tabs.map((t) => {
        const prof = profiles.find((p) => p.id === t.profileId);
        return (
          <div
            key={t.id}
            className={`tab ${t.id === activeTabId ? "active" : ""}`}
            onClick={() => setActiveTab(t.id)}
            onAuxClick={(e) => {
              if (e.button === 1) closeTab(t.id);
            }}
            title={prof ? `${t.title} — ${prof.name}` : t.title}
          >
            {t.running && <span className="spinner" />}
            <span>{t.title}</span>
            {prof && <span style={{ color: "var(--text-dim)", fontSize: 11 }}>{prof.name}</span>}
            <button
              className="close"
              onClick={(e) => {
                e.stopPropagation();
                closeTab(t.id);
              }}
              title="Close tab"
            >
              ×
            </button>
          </div>
        );
      })}
      <button
        className="new-tab"
        onClick={() => setConnectionPicker(true)}
        title="New console tab (⌘T) — pick a connection"
      >
        +
      </button>
    </div>
  );
}
