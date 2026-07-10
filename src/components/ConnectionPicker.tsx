import { useStore } from "../store";
import { DbIcon, engineOf } from "./DbIcons";

/** Shown when the user hits "+" on the tab bar: pick which connection the
 * new console should use. */
export default function ConnectionPicker() {
  const profiles = useStore((s) => s.profiles);
  const sessions = useStore((s) => s.sessions);
  const setConnectionPicker = useStore((s) => s.setConnectionPicker);
  const addTab = useStore((s) => s.addTab);
  const openConnectionModal = useStore((s) => s.openConnectionModal);

  return (
    <div className="modal-backdrop" onClick={() => setConnectionPicker(false)}>
      <div className="modal" style={{ width: 440 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>New console — pick a connection</span>
          <button className="ghost small" onClick={() => setConnectionPicker(false)}>
            ✕
          </button>
        </div>
        <div className="modal-body">
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
        </div>
      </div>
    </div>
  );
}
