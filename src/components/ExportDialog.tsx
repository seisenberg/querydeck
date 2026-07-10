import { useEffect, useState } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { api, errMessage, uid } from "../api";
import { ExportTarget, useStore } from "../store";

type Fmt = "csv" | "tsv" | "json" | "sql";

const EXT: Record<Fmt, string> = { csv: "csv", tsv: "tsv", json: "json", sql: "sql" };

export default function ExportDialog({ target }: { target: ExportTarget }) {
  const closeModals = useStore((s) => s.closeModals);
  const [format, setFormat] = useState<Fmt>("csv");
  const [includeHeader, setIncludeHeader] = useState(true);
  const [limit, setLimit] = useState(0);
  const [sql, setSql] = useState(target.sql);
  const [running, setRunning] = useState(false);
  const [rowsDone, setRowsDone] = useState(0);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [jobId] = useState(uid());

  useEffect(() => {
    const un = listen<{ jobId: string; rowsDone: number }>("job:progress", (ev) => {
      if (ev.payload.jobId === jobId) setRowsDone(ev.payload.rowsDone);
    });
    return () => {
      void un.then((f) => f());
    };
  }, [jobId]);

  const run = async () => {
    const path = await saveDialog({
      title: "Export to file",
      defaultPath: `${target.suggestedName}.${EXT[format]}`,
      filters: [{ name: format.toUpperCase(), extensions: [EXT[format]] }],
    });
    if (!path) return;
    setRunning(true);
    setMessage(null);
    setRowsDone(0);
    try {
      const res = await api.exportRun({
        profileId: target.profileId,
        jobId,
        sql,
        format,
        path,
        includeHeader,
        limit,
        tableName: target.suggestedName,
      });
      setMessage({
        ok: true,
        text: `Exported ${res.rowsExported.toLocaleString()} rows in ${(res.elapsedMs / 1000).toFixed(1)}s → ${path}`,
      });
    } catch (e) {
      setMessage({ ok: false, text: errMessage(e) });
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={running ? undefined : closeModals}>
      <div className="modal" style={{ width: 620 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>Export</span>
          {!running && (
            <button className="ghost small" onClick={closeModals}>
              ✕
            </button>
          )}
        </div>
        <div className="modal-body">
          <div className="form-grid">
            <label>Query</label>
            <textarea
              rows={4}
              value={sql}
              onChange={(e) => setSql(e.target.value)}
              style={{ fontFamily: "var(--mono)", fontSize: 12 }}
            />
            <label>Format</label>
            <div className="form-row-inline">
              {(["csv", "tsv", "json", "sql"] as Fmt[]).map((f) => (
                <label key={f}>
                  <input
                    type="radio"
                    checked={format === f}
                    onChange={() => setFormat(f)}
                  />{" "}
                  {f.toUpperCase()}
                </label>
              ))}
            </div>
            {(format === "csv" || format === "tsv") && (
              <>
                <label />
                <label>
                  <input
                    type="checkbox"
                    checked={includeHeader}
                    onChange={(e) => setIncludeHeader(e.target.checked)}
                  />{" "}
                  include header row
                </label>
              </>
            )}
            <label>Row limit</label>
            <div className="form-row-inline">
              <input
                type="number"
                min={0}
                style={{ width: 130 }}
                value={limit}
                onChange={(e) => setLimit(Math.max(0, Number(e.target.value)))}
              />
              <span className="hint">0 = export everything (streams from the database)</span>
            </div>
          </div>

          {running && (
            <p style={{ marginTop: 12 }}>
              <span className="spinner" /> exporting… {rowsDone.toLocaleString()} rows{" "}
              <button className="small danger" onClick={() => void api.jobCancel(jobId)}>
                Cancel
              </button>
            </p>
          )}
          {message && (
            <p className={message.ok ? "msg-ok" : "msg-err"} style={{ marginTop: 12 }}>
              {message.text}
            </p>
          )}
        </div>
        <div className="modal-footer">
          <span className="sep" />
          <button onClick={closeModals} disabled={running}>
            Close
          </button>
          <button className="primary" onClick={() => void run()} disabled={running || !sql.trim()}>
            Export…
          </button>
        </div>
      </div>
    </div>
  );
}
