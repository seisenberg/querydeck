import { useEffect, useMemo, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  ClickhouseParams,
  ConnectionProfile,
  DriverKind,
  DriverPackage,
  JdbcParams,
  MssqlParams,
  MysqlParams,
  OdbcInventory,
  OdbcParams,
  PostgresParams,
  SqliteParams,
  TunnelConfig,
  api,
  errMessage,
  uid,
} from "../api";
import { useStore } from "../store";

const DRIVER_LABELS: Record<DriverKind, string> = {
  sqlite: "SQLite",
  postgres: "PostgreSQL",
  redshift: "Redshift",
  mysql: "MySQL",
  mariadb: "MariaDB",
  mssql: "SQL Server",
  clickhouse: "ClickHouse",
  odbc: "ODBC",
  jdbc: "JDBC",
};

function defaultParams(kind: DriverKind): ConnectionProfile["params"] {
  switch (kind) {
    case "sqlite":
      return { path: "" } as SqliteParams;
    case "postgres":
      return {
        host: "localhost",
        port: 5432,
        user: "postgres",
        password: "",
        database: "postgres",
        ssl_mode: "prefer",
        accept_invalid_certs: false,
      } as PostgresParams;
    case "redshift":
      return {
        host: "",
        port: 5439,
        user: "awsuser",
        password: "",
        database: "dev",
        ssl_mode: "require",
        accept_invalid_certs: false,
      } as PostgresParams;
    case "mysql":
    case "mariadb":
      return {
        host: "localhost",
        port: 3306,
        user: "root",
        password: "",
        database: "",
        use_ssl: false,
        accept_invalid_certs: false,
      } as MysqlParams;
    case "mssql":
      return {
        host: "localhost",
        port: 1433,
        user: "sa",
        password: "",
        database: "",
        encrypt: true,
        accept_invalid_certs: false,
      } as MssqlParams;
    case "clickhouse":
      return {
        host: "localhost",
        port: 8123,
        user: "default",
        password: "",
        database: "",
        use_https: false,
        accept_invalid_certs: false,
      } as ClickhouseParams;
    case "odbc":
      return { connection_string: "Driver={};Server=;Database=;" } as OdbcParams;
    case "jdbc":
      return {
        url: "jdbc:",
        driver_class: "",
        jars: [],
        user: "",
        password: "",
      } as JdbcParams;
  }
}

/** Drivers configured with host/port/database/user/password. */
function isServerDriver(d: DriverKind): boolean {
  return (
    d === "postgres" ||
    d === "redshift" ||
    d === "mysql" ||
    d === "mariadb" ||
    d === "mssql" ||
    d === "clickhouse"
  );
}

export default function ConnectionModal() {
  const { editing } = useStore((s) => s.connectionModal);
  const closeConnectionModal = useStore((s) => s.closeConnectionModal);
  const loadProfiles = useStore((s) => s.loadProfiles);
  const loadClosedConsoles = useStore((s) => s.loadClosedConsoles);
  const env = useStore((s) => s.env);
  const setDriversModal = useStore((s) => s.setDriversModal);
  const setTunnelsModal = useStore((s) => s.setTunnelsModal);
  const showToast = useStore((s) => s.showToast);

  const [driver, setDriver] = useState<DriverKind>(editing?.driver ?? "sqlite");
  const [name, setName] = useState(editing?.name ?? "");
  const [sshTunnel, setSshTunnel] = useState<string>(editing?.sshTunnel ?? "");
  const [tunnels, setTunnels] = useState<TunnelConfig[]>([]);
  const [odbcInv, setOdbcInv] = useState<OdbcInventory | null>(null);
  const [jdbcPresets, setJdbcPresets] = useState<DriverPackage[]>([]);
  const [params, setParams] = useState<ConnectionProfile["params"]>(
    editing?.params ?? defaultParams(editing?.driver ?? "sqlite"),
  );
  const [testMsg, setTestMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  useEffect(() => {
    api.tunnelsList().then(setTunnels).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (driver === "odbc" && !odbcInv) {
      api.odbcInventory().then(setOdbcInv).catch(() => undefined);
    }
    if (driver === "jdbc" && jdbcPresets.length === 0) {
      api.driversRegistry().then(setJdbcPresets).catch(() => undefined);
    }
  }, [driver, odbcInv, jdbcPresets.length]);

  const profile = useMemo<ConnectionProfile>(
    () => ({
      id: editing?.id || uid(),
      name:
        name.trim() ||
        (driver === "sqlite"
          ? (params as SqliteParams).path.split("/").pop() || "sqlite"
          : driver === "odbc"
            ? "odbc"
            : driver === "jdbc"
              ? "jdbc"
              : `${(params as PostgresParams).database ?? ""}@${(params as PostgresParams).host ?? ""}`),
      driver,
      params,
      sshTunnel: sshTunnel || null,
    }),
    [editing, name, driver, params, sshTunnel],
  );

  const deleteProfile = async () => {
    if (!editing) return;
    setConfirmingDelete(false);
    setBusy(true);
    try {
      await api.profileDelete(editing.id);
      // Its consoles are gone with it — close any that are open.
      useStore.getState().purgeProfileState(editing.id);
      await loadProfiles();
      await loadClosedConsoles();
      closeConnectionModal();
      showToast(`Connection "${editing.name}" deleted`);
    } catch (e) {
      setTestMsg({ ok: false, text: errMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  const pickDriver = (k: DriverKind) => {
    setDriver(k);
    setParams(defaultParams(k));
    setTestMsg(null);
  };

  const set = (patch: Partial<ConnectionProfile["params"]>) =>
    setParams((p) => ({ ...p, ...patch }) as ConnectionProfile["params"]);

  const test = async () => {
    setBusy(true);
    setTestMsg(null);
    try {
      const msg = await api.connectionTest(profile);
      setTestMsg({ ok: true, text: msg });
    } catch (e) {
      setTestMsg({ ok: false, text: errMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    setBusy(true);
    try {
      await api.profileSave(profile);
      await loadProfiles();
      closeConnectionModal();
      // A brand-new connection goes straight to work: open its first console.
      if (!editing) useStore.getState().addTab(profile.id);
    } catch (e) {
      setTestMsg({ ok: false, text: errMessage(e) });
    } finally {
      setBusy(false);
    }
  };

  const pickFile = async (patchKey: "path") => {
    const file = await openDialog({ multiple: false, title: "Choose database file" });
    if (typeof file === "string") set({ [patchKey]: file } as never);
  };

  const pickJars = async () => {
    // Default to the folder QueryDeck downloads driver jars into.
    const defaultPath = await api.driversDir().catch(() => undefined);
    const files = await openDialog({
      multiple: true,
      title: "Choose driver jar(s)",
      defaultPath,
      filters: [{ name: "Java archive", extensions: ["jar"] }],
    });
    if (Array.isArray(files) && files.length) {
      set({ jars: [...(params as JdbcParams).jars, ...files] } as never);
    } else if (typeof files === "string") {
      set({ jars: [...(params as JdbcParams).jars, files] } as never);
    }
  };

  const p = params as never;

  return (
    <div className="modal-backdrop" onClick={closeConnectionModal}>
      <div className="modal" style={{ width: 620 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>{editing ? "Edit connection" : "New connection"}</span>
          <button className="ghost small" onClick={closeConnectionModal}>
            ✕
          </button>
        </div>
        <div className="modal-body">
          {!editing && (
            <div className="driver-picker">
              {(Object.keys(DRIVER_LABELS) as DriverKind[]).map((k) => (
                <button
                  key={k}
                  className={driver === k ? "selected" : ""}
                  onClick={() => pickDriver(k)}
                >
                  {DRIVER_LABELS[k]}
                </button>
              ))}
            </div>
          )}

          <div className="form-grid">
            <label>Name</label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="display name (optional)"
            />

            {driver === "sqlite" && (
              <>
                <label>Database file</label>
                <div className="form-row-inline">
                  <input
                    style={{ flex: 1 }}
                    value={(params as SqliteParams).path}
                    onChange={(e) => set({ path: e.target.value } as never)}
                    placeholder="/path/to/database.db (created if missing)"
                  />
                  <button onClick={() => void pickFile("path")}>Browse…</button>
                </div>
              </>
            )}

            {isServerDriver(driver) && (
              <>
                <label>Host</label>
                <div className="form-row-inline">
                  <input
                    style={{ flex: 1 }}
                    value={(p as PostgresParams).host}
                    onChange={(e) => set({ host: e.target.value } as never)}
                    placeholder={
                      driver === "redshift"
                        ? "cluster.xxxxxx.region.redshift.amazonaws.com"
                        : ""
                    }
                  />
                  <label>Port</label>
                  <input
                    style={{ width: 90 }}
                    type="number"
                    value={(p as PostgresParams).port}
                    onChange={(e) => set({ port: Number(e.target.value) } as never)}
                  />
                </div>
                <label>Database</label>
                <input
                  value={(p as PostgresParams).database}
                  onChange={(e) => set({ database: e.target.value } as never)}
                  placeholder={
                    driver === "postgres" || driver === "redshift" ? "" : "(optional)"
                  }
                />
                <label>User</label>
                <input
                  value={(p as PostgresParams).user}
                  onChange={(e) => set({ user: e.target.value } as never)}
                />
                <label>Password</label>
                <input
                  type="password"
                  value={(p as PostgresParams).password}
                  onChange={(e) => set({ password: e.target.value } as never)}
                />
                <label>SSL</label>
                <div className="form-row-inline">
                  {(driver === "postgres" || driver === "redshift") && (
                    <select
                      value={(p as PostgresParams).ssl_mode}
                      onChange={(e) => set({ ssl_mode: e.target.value } as never)}
                    >
                      <option value="disable">disable</option>
                      <option value="prefer">prefer</option>
                      <option value="require">require</option>
                    </select>
                  )}
                  {(driver === "mysql" || driver === "mariadb") && (
                    <label>
                      <input
                        type="checkbox"
                        checked={(p as MysqlParams).use_ssl}
                        onChange={(e) => set({ use_ssl: e.target.checked } as never)}
                      />{" "}
                      use TLS
                    </label>
                  )}
                  {driver === "mssql" && (
                    <label>
                      <input
                        type="checkbox"
                        checked={(p as MssqlParams).encrypt}
                        onChange={(e) => set({ encrypt: e.target.checked } as never)}
                      />{" "}
                      encrypt connection
                    </label>
                  )}
                  {driver === "clickhouse" && (
                    <label>
                      <input
                        type="checkbox"
                        checked={(p as ClickhouseParams).use_https}
                        onChange={(e) => set({ use_https: e.target.checked } as never)}
                      />{" "}
                      HTTPS (port 8443)
                    </label>
                  )}
                  <label>
                    <input
                      type="checkbox"
                      checked={(p as PostgresParams).accept_invalid_certs}
                      onChange={(e) =>
                        set({ accept_invalid_certs: e.target.checked } as never)
                      }
                    />{" "}
                    trust self-signed certs
                  </label>
                </div>
              </>
            )}

            {driver === "odbc" && (
              <>
                {odbcInv && (odbcInv.drivers.length > 0 || odbcInv.dsns.length > 0) && (
                  <>
                    <label>Installed</label>
                    <div className="form-row-inline">
                      {odbcInv.drivers.length > 0 && (
                        <select
                          value=""
                          onChange={(e) => {
                            if (e.target.value) {
                              set({
                                connection_string: `Driver={${e.target.value}};Server=localhost;Port=;Database=;Uid=;Pwd=;`,
                              } as never);
                            }
                          }}
                          title="Installed ODBC drivers (from the driver manager)"
                        >
                          <option value="">— insert driver… —</option>
                          {odbcInv.drivers.map((d) => (
                            <option key={d} value={d}>
                              {d}
                            </option>
                          ))}
                        </select>
                      )}
                      {odbcInv.dsns.length > 0 && (
                        <select
                          value=""
                          onChange={(e) => {
                            if (e.target.value) {
                              set({ connection_string: `DSN=${e.target.value};` } as never);
                            }
                          }}
                          title="Configured data sources"
                        >
                          <option value="">— use DSN… —</option>
                          {odbcInv.dsns.map(([n, d]) => (
                            <option key={n} value={n}>
                              {n} ({d})
                            </option>
                          ))}
                        </select>
                      )}
                    </div>
                  </>
                )}
                <label>Connection string</label>
                <textarea
                  rows={3}
                  value={(p as OdbcParams).connection_string}
                  onChange={(e) => set({ connection_string: e.target.value } as never)}
                  placeholder={"Driver={PostgreSQL Unicode};Server=localhost;Port=5432;Database=mydb;Uid=me;Pwd=secret;\nor DSN=my_dsn;"}
                />
              </>
            )}

            {driver === "jdbc" && (
              <>
                {jdbcPresets.length > 0 && (
                  <>
                    <label>Preset</label>
                    <select
                      value=""
                      onChange={(e) => {
                        const pkg = jdbcPresets.find((d) => d.id === e.target.value);
                        if (!pkg) return;
                        set({
                          url: pkg.urlTemplate
                            .replace("{host}", "localhost")
                            .replace("{port}", String(pkg.defaultPort || 0)),
                          driver_class: pkg.driverClass,
                          jars: pkg.path ? [pkg.path] : (params as JdbcParams).jars,
                        } as never);
                      }}
                      title="Prefill URL template and driver class for a known engine"
                    >
                      <option value="">— pick an engine… —</option>
                      {jdbcPresets.map((d) => (
                        <option key={d.id} value={d.id}>
                          {d.title}
                          {d.downloaded ? " ✓" : " (driver not downloaded)"}
                        </option>
                      ))}
                    </select>
                  </>
                )}
                <label>JDBC URL</label>
                <input
                  value={(p as JdbcParams).url}
                  onChange={(e) => set({ url: e.target.value } as never)}
                  placeholder="jdbc:postgresql://localhost:5432/mydb"
                />
                <label>Driver class</label>
                <input
                  value={(p as JdbcParams).driver_class}
                  onChange={(e) => set({ driver_class: e.target.value } as never)}
                  placeholder="org.postgresql.Driver"
                />
                <label>Driver jars</label>
                <div>
                  {(p as JdbcParams).jars.map((j, i) => (
                    <div key={i} className="form-row-inline" style={{ marginBottom: 4 }}>
                      <span
                        className="hint"
                        style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}
                        title={j}
                      >
                        {j}
                      </span>
                      <button
                        className="small ghost danger"
                        onClick={() =>
                          set({
                            jars: (p as JdbcParams).jars.filter((_, k) => k !== i),
                          } as never)
                        }
                      >
                        remove
                      </button>
                    </div>
                  ))}
                  <div className="form-row-inline">
                    <button onClick={() => void pickJars()}>Add jar…</button>
                    <button
                      onClick={() => {
                        setDriversModal(true);
                      }}
                    >
                      Get drivers…
                    </button>
                  </div>
                </div>
                <label>User</label>
                <input
                  value={(p as JdbcParams).user}
                  onChange={(e) => set({ user: e.target.value } as never)}
                />
                <label>Password</label>
                <input
                  type="password"
                  value={(p as JdbcParams).password}
                  onChange={(e) => set({ password: e.target.value } as never)}
                />
              </>
            )}
            <label>SSH tunnel</label>
            <div className="form-row-inline">
              <select value={sshTunnel} onChange={(e) => setSshTunnel(e.target.value)}>
                <option value="">— none —</option>
                {tunnels.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.name || t.sshHost} (127.0.0.1:{t.localPort} → {t.remoteHost}:{t.remotePort})
                  </option>
                ))}
              </select>
              <button
                className="small"
                onClick={() => {
                  setTunnelsModal(true);
                }}
                title="Manage SSH tunnels"
              >
                Manage…
              </button>
            </div>
            {sshTunnel && (
              <>
                <label />
                <span className="hint">
                  The tunnel is started automatically before connecting — point host/port (or
                  URL/connection string) at the tunnel's local port.
                </span>
              </>
            )}
          </div>

          {driver === "odbc" && env && !env.odbcAvailable && (
            <p className="hint" style={{ marginTop: 10, color: "var(--warn)" }}>
              {env.odbcDetail}
            </p>
          )}
          {driver === "jdbc" && env && !env.jvmAvailable && (
            <p className="hint" style={{ marginTop: 10, color: "var(--warn)" }}>
              {env.jvmDetail}
            </p>
          )}

          {testMsg && (
            <p className={testMsg.ok ? "msg-ok" : "msg-err"} style={{ marginTop: 10 }}>
              {testMsg.text}
            </p>
          )}
        </div>
        <div className="modal-footer">
          <button onClick={() => void test()} disabled={busy}>
            {busy ? "…" : "Test connection"}
          </button>
          {editing && (
            <span className="popover-anchor">
              <button
                className="danger"
                onClick={() => setConfirmingDelete((v) => !v)}
                disabled={busy}
              >
                Delete…
              </button>
              {confirmingDelete && (
                <div className="popover popover-above" onClick={(e) => e.stopPropagation()}>
                  <div className="popover-title">Delete “{editing.name}”?</div>
                  <p className="hint" style={{ margin: "6px 0 10px" }}>
                    Its consoles, queries and logs will be permanently removed. This
                    cannot be undone.
                  </p>
                  <div className="form-row-inline" style={{ justifyContent: "flex-end" }}>
                    <button className="small" onClick={() => setConfirmingDelete(false)}>
                      Cancel
                    </button>
                    <button
                      className="small danger-solid"
                      onClick={() => void deleteProfile()}
                      disabled={busy}
                    >
                      Delete
                    </button>
                  </div>
                </div>
              )}
            </span>
          )}
          <span className="sep" />
          <button onClick={closeConnectionModal}>Cancel</button>
          <button className="primary" onClick={() => void save()} disabled={busy}>
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
