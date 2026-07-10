import { useEffect, useState } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { api, errMessage, uid } from "../api";

type Fmt = "csv" | "xlsx" | "sql";

const EXT: Record<Fmt, string> = { csv: "csv", xlsx: "xlsx", sql: "sql" };

interface Props {
  profileId: string;
  sql: string;
  tableName: string;
  database?: string | null;
}

/** Export options + runner. Lives on the Export tab of a console (and in the
 * table export modal). */
export default function ExportPane({ profileId, sql, tableName, database }: Props) {
  const [format, setFormat] = useState<Fmt>("csv");
  const [includeHeader, setIncludeHeader] = useState(true);
  const [includeCreate, setIncludeCreate] = useState(false);
  const [insertStyle, setInsertStyle] = useState<"single" | "multiline">("single");
  const [batchLimit, setBatchLimit] = useState<number>(0);
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
      defaultPath: `${tableName || "result"}.${EXT[format]}`,
      filters: [{ name: format.toUpperCase(), extensions: [EXT[format]] }],
    });
    if (!path) return;
    setRunning(true);
    setMessage(null);
    setRowsDone(0);
    try {
      const res = await api.exportRun({
        profileId,
        jobId,
        sql,
        format,
        path,
        includeHeader,
        limit: 0,
        tableName: tableName || "exported_table",
        database: database ?? null,
        includeCreate: format === "sql" ? includeCreate : false,
        insertStyle,
        batchLimit: insertStyle === "multiline" ? batchLimit : 0,
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

  if (!sql.trim()) {
    return (
      <div className="placeholder">
        <div>Nothing to export yet.</div>
        <div className="hint">Run a query first — the export re-runs it and streams every row to a file.</div>
      </div>
    );
  }

  return (
    <div className="export-pane">
      <div className="export-sql hint" title={sql}>
        {sql.replace(/\s+/g, " ").slice(0, 200)}
      </div>

      <div className="export-formats">
        {(["csv", "xlsx", "sql"] as Fmt[]).map((f) => (
          <label key={f} className={`export-fmt ${format === f ? "selected" : ""}`}>
            <input type="radio" checked={format === f} onChange={() => setFormat(f)} />
            {f === "csv" ? "CSV" : f === "xlsx" ? "Excel" : "SQL inserts"}
          </label>
        ))}
      </div>

      <div className="export-options">
        {(format === "csv" || format === "xlsx") && (
          <label>
            <input
              type="checkbox"
              checked={includeHeader}
              onChange={(e) => setIncludeHeader(e.target.checked)}
            />{" "}
            headers
          </label>
        )}
        {format === "sql" && (
          <>
            <label>
              <input
                type="checkbox"
                checked={includeCreate}
                onChange={(e) => setIncludeCreate(e.target.checked)}
              />{" "}
              table create statement
            </label>
            <span className="export-sub">
              insert style:{" "}
              <label>
                <input
                  type="radio"
                  checked={insertStyle === "single"}
                  onChange={() => setInsertStyle("single")}
                />{" "}
                single
              </label>
              <label>
                <input
                  type="radio"
                  checked={insertStyle === "multiline"}
                  onChange={() => setInsertStyle("multiline")}
                />{" "}
                multiline
              </label>
              {insertStyle === "multiline" && (
                <label>
                  batch limit{" "}
                  <input
                    type="number"
                    min={0}
                    style={{ width: 90 }}
                    value={batchLimit}
                    onChange={(e) => setBatchLimit(Math.max(0, Number(e.target.value)))}
                    title="Start a new INSERT statement every N rows (0 = one statement)"
                  />
                </label>
              )}
            </span>
          </>
        )}
      </div>

      <div className="form-row-inline" style={{ marginTop: 10 }}>
        <button className="primary" onClick={() => void run()} disabled={running}>
          Export…
        </button>
        {running && (
          <span>
            <span className="spinner" /> {rowsDone.toLocaleString()} rows{" "}
            <button className="small danger" onClick={() => void api.jobCancel(jobId)}>
              Cancel
            </button>
          </span>
        )}
      </div>
      {message && (
        <p className={message.ok ? "msg-ok" : "msg-err"} style={{ marginTop: 10 }}>
          {message.text}
        </p>
      )}
    </div>
  );
}
