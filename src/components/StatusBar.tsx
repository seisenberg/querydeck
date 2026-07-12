import { useStore } from "../store";

/** Application work state for the statusbar icon. */
function ActivityIcon() {
  const activity = useStore((s) => s.activity);
  const setActivityModal = useStore((s) => s.setActivityModal);

  const hasQuery = activity.some((a) => a.kind === "query");
  const hasBg = activity.some((a) => a.kind === "background");
  const state = hasQuery && hasBg ? "both" : hasQuery ? "query" : hasBg ? "bg" : "idle";
  const title =
    state === "idle"
      ? "Idle — click for background activity"
      : state === "query"
        ? "Running your queries — click for details"
        : state === "bg"
          ? "Background work (schema loads, jobs) — click for details"
          : "Queries + background work — click for details";

  return (
    <button
      className={`ghost small activity-icon ${state}`}
      onClick={() => setActivityModal(true)}
      title={title}
    >
      <svg width="12" height="12" viewBox="0 0 12 12">
        {/* base dot: idle gray, colored by state */}
        <circle
          cx="6"
          cy="6"
          r="4.5"
          fill={
            state === "idle"
              ? "var(--icon-muted)"
              : state === "query"
                ? "var(--accent)"
                : state === "bg"
                  ? "var(--warn)"
                  : "var(--accent)"
          }
        />
        {state === "both" && (
          <circle cx="9.5" cy="2.8" r="2.4" fill="var(--warn)" stroke="var(--bg-panel)" strokeWidth="0.8" />
        )}
        {state !== "idle" && (
          <circle cx="6" cy="6" r="4.5" fill="none" stroke="currentColor" strokeOpacity="0.25" />
        )}
      </svg>
      {activity.length > 0 ? ` ${activity.length} running` : ""}
    </button>
  );
}

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
      <ActivityIcon />
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
