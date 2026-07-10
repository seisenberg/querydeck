import { useEffect, useRef, useState } from "react";
import { HistoryEntry, api, errMessage } from "../api";
import { useStore } from "../store";

export default function HistoryModal() {
  const setHistoryModal = useStore((s) => s.setHistoryModal);
  const profiles = useStore((s) => s.profiles);
  const addTab = useStore((s) => s.addTab);
  const showToast = useStore((s) => s.showToast);

  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const [filter, setFilter] = useState("");
  const [profileId, setProfileId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const debounce = useRef<number | undefined>(undefined);

  const load = (f: string, pid: string) => {
    api
      .historyList(500, f || undefined, pid || undefined)
      .then((e) => {
        setEntries(e);
        setError(null);
      })
      .catch((e) => setError(errMessage(e)));
  };

  useEffect(() => {
    load("", "");
  }, []);

  const onFilter = (f: string, pid: string) => {
    setFilter(f);
    setProfileId(pid);
    window.clearTimeout(debounce.current);
    debounce.current = window.setTimeout(() => load(f, pid), 200);
  };

  const openInConsole = (e: HistoryEntry) => {
    const profileStillExists = profiles.some((p) => p.id === e.profileId);
    addTab(profileStillExists ? e.profileId : null, e.sql.endsWith(";") ? e.sql : `${e.sql};`);
    setHistoryModal(false);
  };

  return (
    <div className="modal-backdrop" onClick={() => setHistoryModal(false)}>
      <div className="modal" style={{ width: 860 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>Query history</span>
          <button className="ghost small" onClick={() => setHistoryModal(false)}>
            ✕
          </button>
        </div>
        <div className="modal-body" style={{ minHeight: 300 }}>
          <div className="form-row-inline" style={{ marginBottom: 10 }}>
            <input
              style={{ flex: 1 }}
              placeholder="Search SQL…"
              value={filter}
              onChange={(e) => onFilter(e.target.value, profileId)}
              autoFocus
            />
            <select value={profileId} onChange={(e) => onFilter(filter, e.target.value)}>
              <option value="">all connections</option>
              {profiles.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
            <button
              className="small danger"
              onClick={() => {
                if (window.confirm("Clear the entire query history?")) {
                  api
                    .historyClear()
                    .then(() => load(filter, profileId))
                    .catch((e) => showToast(errMessage(e)));
                }
              }}
            >
              Clear…
            </button>
          </div>

          {error && <div className="msg-err">{error}</div>}
          {entries === null && !error && (
            <div className="placeholder" style={{ minHeight: 120 }}>
              <span className="spinner" />
            </div>
          )}
          {entries !== null && entries.length === 0 && (
            <div className="placeholder" style={{ minHeight: 120 }}>
              <div>No history{filter || profileId ? " matching the filter" : " yet"}.</div>
              <div className="hint">Every executed statement is recorded here.</div>
            </div>
          )}
          {entries !== null && entries.length > 0 && (
            <div className="history-list">
              {entries.map((e, i) => (
                <div key={i} className="history-row">
                  <div className="history-meta">
                    <span className="log-ts">{e.ts}</span>
                    <span className="history-profile">{e.profileName}</span>
                    <span className={`log-status ${e.kind}`}>
                      {e.kind === "rows" && `${(e.rowCount ?? 0).toLocaleString()} rows`}
                      {e.kind === "affected" && `${(e.affected ?? 0).toLocaleString()} affected`}
                      {e.kind === "ok" && "ok"}
                      {e.kind === "error" && "error"}
                    </span>
                    <span className="log-elapsed">{e.elapsedMs} ms</span>
                    <span className="spacer" style={{ flex: 1 }} />
                    <button
                      className="small ghost"
                      onClick={() => {
                        void navigator.clipboard.writeText(e.sql);
                        showToast("SQL copied");
                      }}
                    >
                      Copy
                    </button>
                    <button className="small" onClick={() => openInConsole(e)}>
                      Open in console
                    </button>
                  </div>
                  <div className="history-sql" title={e.error ? `${e.sql}\n\n${e.error}` : e.sql}>
                    {e.sql.replace(/\s+/g, " ").slice(0, 400)}
                  </div>
                  {e.error && <div className="log-error">{e.error}</div>}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
