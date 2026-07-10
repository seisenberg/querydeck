import { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { TunnelConfig, api, errMessage, uid } from "../api";
import { useStore } from "../store";

function emptyTunnel(): TunnelConfig {
  return {
    id: uid(),
    name: "",
    sshHost: "",
    sshPort: 22,
    sshUser: "",
    keyPath: "",
    localPort: 15432,
    remoteHost: "localhost",
    remotePort: 5432,
  };
}

/** SSH tunnel management: define local forwards through an SSH host; database
 * connections reference a tunnel and target its local port. */
export default function TunnelsModal() {
  const setTunnelsModal = useStore((s) => s.setTunnelsModal);
  const showToast = useStore((s) => s.showToast);
  const [tunnels, setTunnels] = useState<TunnelConfig[]>([]);
  const [status, setStatus] = useState<Record<string, boolean>>({});
  const [editing, setEditing] = useState<TunnelConfig | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setTunnels(await api.tunnelsList());
      const st = await api.tunnelsStatus();
      setStatus(Object.fromEntries(st.map((s) => [s.id, s.running])));
    } catch (e) {
      showToast(errMessage(e));
    }
  };

  useEffect(() => {
    void refresh();
    const iv = window.setInterval(() => void refresh(), 4000);
    return () => window.clearInterval(iv);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const save = async () => {
    if (!editing) return;
    try {
      await api.tunnelSave(editing);
      setEditing(null);
      await refresh();
    } catch (e) {
      setError(errMessage(e));
    }
  };

  const start = async (id: string) => {
    setBusy(id);
    setError(null);
    try {
      await api.tunnelStart(id);
    } catch (e) {
      setError(errMessage(e));
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  return (
    <div className="modal-backdrop" onClick={() => setTunnelsModal(false)}>
      <div className="modal" style={{ width: 680 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>SSH tunnels</span>
          <button className="ghost small" onClick={() => setTunnelsModal(false)}>
            ✕
          </button>
        </div>
        <div className="modal-body">
          <p className="hint" style={{ marginBottom: 10 }}>
            A tunnel forwards <code>127.0.0.1:&lt;local port&gt;</code> to a host reachable from
            the SSH server. Point your connection at the local port and select the tunnel in the
            connection settings — it is started automatically before connecting. Auth uses your
            SSH keys/agent (BatchMode); password prompts are not supported.
          </p>

          {tunnels.map((t) => (
            <div className="pkg-row" key={t.id}>
              <span
                className="tunnel-dot"
                style={{ background: status[t.id] ? "#39d353" : "#5b6270" }}
                title={status[t.id] ? "running" : "stopped"}
              />
              <div className="info">
                <div className="title">{t.name || t.sshHost}</div>
                <div className="sub">
                  127.0.0.1:{t.localPort} → {t.remoteHost}:{t.remotePort} via {t.sshUser}@
                  {t.sshHost}:{t.sshPort}
                </div>
              </div>
              {status[t.id] ? (
                <button
                  className="small"
                  onClick={() => void api.tunnelStop(t.id).then(refresh)}
                >
                  Stop
                </button>
              ) : (
                <button className="small primary" disabled={busy === t.id} onClick={() => void start(t.id)}>
                  {busy === t.id ? "Starting…" : "Start"}
                </button>
              )}
              <button className="small" onClick={() => setEditing({ ...t })}>
                Edit
              </button>
              <button
                className="small ghost danger"
                onClick={() => {
                  if (window.confirm(`Delete tunnel "${t.name || t.sshHost}"?`)) {
                    void api.tunnelDelete(t.id).then(refresh);
                  }
                }}
              >
                ✕
              </button>
            </div>
          ))}

          {!editing && (
            <button style={{ marginTop: 10 }} onClick={() => setEditing(emptyTunnel())}>
              + Add tunnel
            </button>
          )}

          {editing && (
            <div className="tunnel-form">
              <div className="form-grid">
                <label>Name</label>
                <input
                  value={editing.name}
                  onChange={(e) => setEditing({ ...editing, name: e.target.value })}
                  placeholder="prod bastion"
                />
                <label>SSH host</label>
                <div className="form-row-inline">
                  <input
                    style={{ flex: 1 }}
                    value={editing.sshHost}
                    onChange={(e) => setEditing({ ...editing, sshHost: e.target.value })}
                    placeholder="bastion.example.com"
                  />
                  <label>port</label>
                  <input
                    type="number"
                    style={{ width: 84 }}
                    value={editing.sshPort}
                    onChange={(e) => setEditing({ ...editing, sshPort: Number(e.target.value) })}
                  />
                </div>
                <label>SSH user</label>
                <input
                  value={editing.sshUser}
                  onChange={(e) => setEditing({ ...editing, sshUser: e.target.value })}
                />
                <label>Key file</label>
                <div className="form-row-inline">
                  <input
                    style={{ flex: 1 }}
                    value={editing.keyPath ?? ""}
                    onChange={(e) => setEditing({ ...editing, keyPath: e.target.value })}
                    placeholder="(optional — agent/config used otherwise)"
                  />
                  <button
                    className="small"
                    onClick={() =>
                      void openDialog({ multiple: false, title: "Choose private key" }).then((f) => {
                        if (typeof f === "string") setEditing((t) => t && { ...t, keyPath: f });
                      })
                    }
                  >
                    Browse…
                  </button>
                </div>
                <label>Local port</label>
                <input
                  type="number"
                  style={{ width: 110 }}
                  value={editing.localPort}
                  onChange={(e) => setEditing({ ...editing, localPort: Number(e.target.value) })}
                />
                <label>Remote target</label>
                <div className="form-row-inline">
                  <input
                    style={{ flex: 1 }}
                    value={editing.remoteHost}
                    onChange={(e) => setEditing({ ...editing, remoteHost: e.target.value })}
                    placeholder="db.internal"
                  />
                  <label>port</label>
                  <input
                    type="number"
                    style={{ width: 84 }}
                    value={editing.remotePort}
                    onChange={(e) => setEditing({ ...editing, remotePort: Number(e.target.value) })}
                  />
                </div>
              </div>
              <div className="form-row-inline" style={{ marginTop: 10 }}>
                <button className="primary" onClick={() => void save()}>
                  Save tunnel
                </button>
                <button onClick={() => setEditing(null)}>Cancel</button>
              </div>
            </div>
          )}

          {error && (
            <p className="msg-err" style={{ marginTop: 10 }}>
              {error}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
