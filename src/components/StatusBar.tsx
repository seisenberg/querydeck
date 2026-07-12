import { useStore } from "../store";

/** Application work state: just the colored dot (no words) — the count and
 * details live in its tooltip and the activity modal. */
function ActivityIcon() {
  const activity = useStore((s) => s.activity);
  const setActivityModal = useStore((s) => s.setActivityModal);

  const hasQuery = activity.some((a) => a.kind === "query");
  const hasBg = activity.some((a) => a.kind === "background");
  const state = hasQuery && hasBg ? "both" : hasQuery ? "query" : hasBg ? "bg" : "idle";
  const n = activity.length;
  const title =
    state === "idle"
      ? "Idle — click for background activity"
      : state === "query"
        ? `Running ${n} quer${n === 1 ? "y" : "ies"} — click for details`
        : state === "bg"
          ? `${n} background job${n === 1 ? "" : "s"} (schema loads, imports/exports) — click for details`
          : `${n} running (queries + background work) — click for details`;

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
      {prof && <span>{`${prof.name} (${prof.driver})`}</span>}
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
      <ActivityIcon />
      {env && (
        <button className="ghost small" onClick={() => setDriversModal(true)}>
          ODBC {env.odbcAvailable ? "✓" : "—"} · Java {env.jvmAvailable ? "✓" : "—"} · Drivers
        </button>
      )}
    </div>
  );
}
