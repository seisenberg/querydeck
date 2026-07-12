import { useEffect, useState } from "react";
import { useStore } from "../store";

/** Click on the statusbar activity icon: everything the backend is doing
 * right now, with a per-connection kill switch for stuck sessions. */
export default function ActivityModal() {
  const activity = useStore((s) => s.activity);
  const profiles = useStore((s) => s.profiles);
  const setActivityModal = useStore((s) => s.setActivityModal);
  const killBackground = useStore((s) => s.killBackground);
  const cancelQuery = useStore((s) => s.cancelQuery);
  const tabs = useStore((s) => s.tabs);

  // Re-render each second so the elapsed column ticks.
  const [, setTick] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => setTick((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, []);

  const profileName = (id: string) => profiles.find((p) => p.id === id)?.name ?? id;
  const elapsed = (startedMs: number) => {
    const s = Math.max(0, Math.round((Date.now() - startedMs) / 1000));
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
  };

  // Connections involved in any current work (for the kill switch), plus
  // any connection with open sessions (a stuck session may show no work).
  const involved = [...new Set(activity.map((a) => a.profileId))];

  return (
    <div className="modal-backdrop" onClick={() => setActivityModal(false)}>
      <div className="modal" style={{ width: 560 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>Background activity</span>
          <button className="ghost small" onClick={() => setActivityModal(false)}>
            ✕
          </button>
        </div>
        <div className="modal-body" style={{ maxHeight: "60vh", overflowY: "auto" }}>
          {activity.length === 0 && (
            <div className="placeholder" style={{ minHeight: 70 }}>
              <div>Nothing running — the app is idle.</div>
            </div>
          )}
          {activity.map((a) => {
            // A user query can be cancelled through its console's session.
            const queryTab =
              a.kind === "query"
                ? tabs.find((t) => t.profileId === a.profileId && t.running)
                : undefined;
            return (
              <div key={a.id} className="activity-row">
                <span className={`activity-kind ${a.kind}`}>
                  {a.kind === "query" ? "query" : "background"}
                </span>
                <span className="activity-label" title={a.label}>
                  {a.label}
                </span>
                <span className="meta">{profileName(a.profileId)}</span>
                <span className="meta">{elapsed(a.startedMs)}</span>
                {queryTab && (
                  <button
                    className="small danger"
                    onClick={() => cancelQuery(queryTab.id)}
                    title="Cancel this query"
                  >
                    Cancel
                  </button>
                )}
              </div>
            );
          })}

          {involved.length > 0 && (
            <>
              <div className="picker-section" style={{ marginTop: 14 }}>
                Kill switch
              </div>
              <p className="hint" style={{ margin: "0 0 8px" }}>
                If something looks stuck or hung, disconnect the connection outright.
                This drops every session on it and clears its schema cache; the next
                use reconnects fresh.
              </p>
              {involved.map((pid) => (
                <div key={pid} className="activity-row">
                  <span className="activity-label">{profileName(pid)}</span>
                  <button className="small danger" onClick={() => void killBackground(pid)}>
                    Disconnect &amp; clear cache
                  </button>
                </div>
              ))}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
