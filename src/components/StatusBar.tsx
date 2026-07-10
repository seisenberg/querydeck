import { useStore } from "../store";

export default function StatusBar() {
  const tabs = useStore((s) => s.tabs);
  const activeTabId = useStore((s) => s.activeTabId);
  const profiles = useStore((s) => s.profiles);
  const env = useStore((s) => s.env);
  const setDriversModal = useStore((s) => s.setDriversModal);

  const tab = tabs.find((t) => t.id === activeTabId);
  const prof = profiles.find((p) => p.id === tab?.profileId);
  const active = tab?.results[tab.activeResult];

  return (
    <div className="statusbar">
      <span>{prof ? `${prof.name} (${prof.driver})` : "no connection"}</span>
      {tab?.running && <span>running…</span>}
      {active && active.kind === "rows" && (
        <span>
          {active.rowCount.toLocaleString()} rows{active.truncated ? " (limited)" : ""} ·{" "}
          {active.elapsedMs} ms
        </span>
      )}
      {active && active.kind === "affected" && (
        <span>
          {active.affected?.toLocaleString()} affected · {active.elapsedMs} ms
        </span>
      )}
      <span className="spring" />
      {env && (
        <button className="ghost small" onClick={() => setDriversModal(true)}>
          ODBC {env.odbcAvailable ? "✓" : "—"} · Java {env.jvmAvailable ? "✓" : "—"} · Drivers
        </button>
      )}
    </div>
  );
}
