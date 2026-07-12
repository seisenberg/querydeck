import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { DriverPackage, api, errMessage } from "../api";
import { useStore } from "../store";

export default function DriverManager() {
  const setDriversModal = useStore((s) => s.setDriversModal);
  const env = useStore((s) => s.env);
  const showToast = useStore((s) => s.showToast);
  const [packages, setPackages] = useState<DriverPackage[]>([]);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [downloading, setDownloading] = useState<Record<string, boolean>>({});
  const [filter, setFilter] = useState("");

  const refresh = () => {
    api.driversRegistry().then(setPackages).catch((e) => showToast(errMessage(e)));
  };

  useEffect(refresh, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const un = listen<{ id: string; done: number; total: number }>("driver:progress", (ev) => {
      const { id, done, total } = ev.payload;
      setProgress((p) => ({ ...p, [id]: total > 0 ? done / total : 0 }));
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const download = async (pkg: DriverPackage) => {
    setDownloading((d) => ({ ...d, [pkg.id]: true }));
    try {
      await api.driverDownload(pkg.id);
      refresh();
    } catch (e) {
      showToast(errMessage(e));
    } finally {
      setDownloading((d) => ({ ...d, [pkg.id]: false }));
    }
  };

  // Downloaded drivers float to the top of their alphabetical order.
  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return packages
      .filter(
        (p) =>
          !q ||
          p.title.toLowerCase().includes(q) ||
          p.driverClass.toLowerCase().includes(q) ||
          p.notes.toLowerCase().includes(q),
      )
      .sort(
        (a, b) =>
          Number(b.downloaded) - Number(a.downloaded) || a.title.localeCompare(b.title),
      );
  }, [packages, filter]);

  const downloadedCount = packages.filter((p) => p.downloaded).length;

  return (
    <div className="modal-backdrop" onClick={() => setDriversModal(false)}>
      <div className="modal" style={{ width: 760 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>
            Database drivers{" "}
            <span className="hint">
              {downloadedCount}/{packages.length} downloaded
            </span>
          </span>
          <button className="ghost small" onClick={() => setDriversModal(false)}>
            ✕
          </button>
        </div>
        <div className="modal-body" style={{ maxHeight: "70vh", overflowY: "auto" }}>
          {env && (
            <>
              <div className={`env-banner ${env.odbcAvailable ? "ok" : "bad"}`}>
                <b>ODBC</b>
                <span className="hint">{env.odbcDetail}</span>
              </div>
              <div
                className={`env-banner ${env.jvmAvailable ? "ok" : "bad"}`}
                style={{ marginBottom: 10 }}
              >
                <b>Java (for JDBC)</b>
                <span className="hint">{env.jvmDetail}</span>
              </div>
            </>
          )}
          <p className="hint" style={{ marginBottom: 8 }}>
            SQLite, PostgreSQL, MySQL/MariaDB, SQL Server, Redshift and ClickHouse have
            built-in native drivers — nothing to download. Everything below is the
            latest vendor JDBC driver, fetched from Maven Central with one click; ODBC
            drivers are installed system-wide by their vendors.
          </p>
          <input
            autoFocus
            placeholder="Filter drivers… (name, class, notes)"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            style={{ width: "100%", marginBottom: 10 }}
          />
          {shown.length === 0 && (
            <div className="placeholder" style={{ minHeight: 60 }}>
              <div>No drivers match “{filter}”.</div>
            </div>
          )}
          {shown.map((pkg) => (
            <div className="pkg-row" key={pkg.id}>
              <div className="info">
                <div className="title">
                  {pkg.downloaded && <span title="Downloaded">✓ </span>}
                  {pkg.title} <span className="hint">{pkg.version}</span>
                </div>
                <div className="sub">
                  {pkg.driverClass} · {pkg.urlTemplate}
                  {pkg.notes ? ` · ${pkg.notes}` : ""}
                </div>
                {downloading[pkg.id] && (
                  <div className="progress-outer" style={{ marginTop: 5 }}>
                    <div
                      className="progress-inner"
                      style={{ width: `${Math.round((progress[pkg.id] ?? 0) * 100)}%` }}
                    />
                  </div>
                )}
              </div>
              {pkg.downloaded ? (
                <UseButton pkg={pkg} />
              ) : (
                <button
                  className="primary small"
                  disabled={!!downloading[pkg.id]}
                  onClick={() => void download(pkg)}
                >
                  {downloading[pkg.id] ? "Downloading…" : "Download"}
                </button>
              )}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function UseButton({ pkg }: { pkg: DriverPackage }) {
  const openConnectionModal = useStore((s) => s.openConnectionModal);
  const setDriversModal = useStore((s) => s.setDriversModal);
  return (
    <button
      className="small"
      title="Create a JDBC connection using this driver"
      onClick={() => {
        setDriversModal(false);
        openConnectionModal(
          {
            id: "",
            name: "",
            driver: "jdbc",
            params: {
              url: pkg.urlTemplate
                .replace("{host}", "localhost")
                .replace("{port}", String(pkg.defaultPort || 0)),
              driver_class: pkg.driverClass,
              jars: pkg.path ? [pkg.path] : [],
              user: "",
              password: "",
            },
          },
          pkg.path ?? undefined,
        );
      }}
    >
      New connection
    </button>
  );
}
