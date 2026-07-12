import { useStore } from "../store";
import { ConsoleFile } from "../api";
import { DbIcon, engineOf } from "./DbIcons";

/** Shown for "+" / ⌘T: open a console — either a new one on a chosen
 * connection, or reopen a saved (closed) console. */
export default function ConnectionPicker() {
  const profiles = useStore((s) => s.profiles);
  const sessions = useStore((s) => s.sessions);
  const closedConsoles = useStore((s) => s.closedConsoles);
  const setConnectionPicker = useStore((s) => s.setConnectionPicker);
  const addTab = useStore((s) => s.addTab);
  const reopenConsole = useStore((s) => s.reopenConsole);
  const openConnectionModal = useStore((s) => s.openConnectionModal);

  const saved: ConsoleFile[] = Object.values(closedConsoles)
    .flat()
    .sort((a, b) => (a.updatedAt < b.updatedAt ? 1 : -1));

  const profileName = (id: string) => profiles.find((p) => p.id === id)?.name ?? "?";
  const profileOf = (id: string) => profiles.find((p) => p.id === id);

  return (
    <div className="modal-backdrop" onClick={() => setConnectionPicker(false)}>
      <div className="modal" style={{ width: 460 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>Open Console — pick a connection</span>
          <button className="ghost small" onClick={() => setConnectionPicker(false)}>
            ✕
          </button>
        </div>
        <div className="modal-body" style={{ maxHeight: "60vh", overflowY: "auto" }}>
          {profiles.length === 0 && (
            <div className="placeholder" style={{ minHeight: 80 }}>
              <div>No connections yet.</div>
              <button
                className="primary"
                onClick={() => {
                  setConnectionPicker(false);
                  openConnectionModal();
                }}
              >
                Add a connection
              </button>
            </div>
          )}
          {profiles.length > 0 && <div className="picker-section">New console on…</div>}
          {profiles.map((p) => (
            <button
              key={p.id}
              className="picker-row"
              onClick={() => {
                setConnectionPicker(false);
                addTab(p.id);
              }}
            >
              <DbIcon engine={engineOf(p)} connected={(sessions[p.id] ?? 0) > 0} />
              <span className="label">{p.name}</span>
              <span className="meta">{p.driver}</span>
            </button>
          ))}
          {saved.length > 0 && (
            <>
              <div className="picker-section">Reopen a saved console</div>
              {saved.map((c) => {
                const prof = profileOf(c.profileId);
                return (
                  <button
                    key={c.id}
                    className="picker-row"
                    onClick={() => {
                      setConnectionPicker(false);
                      reopenConsole(c);
                    }}
                  >
                    {prof && (
                      <DbIcon
                        engine={engineOf(prof)}
                        connected={(sessions[c.profileId] ?? 0) > 0}
                      />
                    )}
                    <span className="label">{c.title}</span>
                    <span className="meta">
                      {profileName(c.profileId)} · {c.updatedAt.slice(0, 16)}
                    </span>
                  </button>
                );
              })}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
