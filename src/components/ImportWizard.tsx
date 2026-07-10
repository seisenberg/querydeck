import { useEffect, useMemo, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { ImportMapping, ImportPreview, api, errMessage, uid } from "../api";
import { ImportTarget, useStore } from "../store";

type Step = "file" | "mapping" | "running" | "done";

interface MappingRow {
  fileIndex: number;
  fileName: string;
  include: boolean;
  column: string;
  dataType: string;
}

const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");

export default function ImportWizard({ target }: { target: ImportTarget }) {
  const closeModals = useStore((s) => s.closeModals);
  const catalogs = useStore((s) => s.catalogs);
  const loadCatalog = useStore((s) => s.loadCatalog);
  const profiles = useStore((s) => s.profiles);

  const profile = profiles.find((p) => p.id === target.profileId);
  const catalog = catalogs[target.profileId];

  const [step, setStep] = useState<Step>("file");
  const [path, setPath] = useState("");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [delimiter, setDelimiter] = useState<string>("");
  const [hasHeader, setHasHeader] = useState<boolean | null>(null);
  const [loadingPreview, setLoadingPreview] = useState(false);

  const [schema, setSchema] = useState(target.schema ?? catalog?.schemas[0]?.name ?? "");
  const [createNew, setCreateNew] = useState(!target.table);
  const [tableName, setTableName] = useState(target.table ?? "");
  const [rows, setRows] = useState<MappingRow[]>([]);
  const [emptyAsNull, setEmptyAsNull] = useState(true);

  const [jobId] = useState(uid());
  const [progress, setProgress] = useState<{ rows: number; bytes: number; total: number }>({
    rows: 0,
    bytes: 0,
    total: 0,
  });
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!catalog) void loadCatalog(target.profileId);
  }, [catalog, loadCatalog, target.profileId]);

  useEffect(() => {
    const un = listen<{ jobId: string; rowsDone: number; bytesDone: number; totalBytes: number }>(
      "job:progress",
      (ev) => {
        if (ev.payload.jobId === jobId) {
          setProgress({
            rows: ev.payload.rowsDone,
            bytes: ev.payload.bytesDone,
            total: ev.payload.totalBytes,
          });
        }
      },
    );
    return () => {
      void un.then((f) => f());
    };
  }, [jobId]);

  const existingTable = useMemo(
    () =>
      catalog?.schemas
        .find((s) => s.name === schema)
        ?.tables.find((t) => t.name === tableName),
    [catalog, schema, tableName],
  );

  const loadPreview = async (
    p: string,
    delimOverride?: string,
    headerOverride?: boolean | null,
  ) => {
    setLoadingPreview(true);
    setError(null);
    try {
      const pv = await api.importPreview(
        p,
        delimOverride || undefined,
        headerOverride == null ? undefined : headerOverride,
      );
      setPreview(pv);
      setDelimiter(pv.delimiter);
      setHasHeader(pv.hasHeader);
      // (re)build mapping rows
      setRows(
        pv.columns.map((c, i) => ({
          fileIndex: i,
          fileName: c,
          include: true,
          column: sanitizeIdent(c),
          dataType: pv.inferredTypes[i] ?? "text",
        })),
      );
      if (!tableName && !target.table) {
        const base = p.split(/[\\/]/).pop()?.replace(/\.[^.]*$/, "") ?? "imported";
        setTableName(sanitizeIdent(base));
      }
    } catch (e) {
      setError(errMessage(e));
      setPreview(null);
    } finally {
      setLoadingPreview(false);
    }
  };

  const pickFile = async () => {
    const f = await openDialog({
      multiple: false,
      title: "Choose CSV / TSV file",
      filters: [
        { name: "Delimited files", extensions: ["csv", "tsv", "txt"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
    if (typeof f === "string") {
      setPath(f);
      await loadPreview(f);
    }
  };

  // Auto-match file columns to existing table columns by normalized name.
  useEffect(() => {
    if (!existingTable || createNew) return;
    setRows((rs) =>
      rs.map((r) => {
        const match = existingTable.columns.find((c) => norm(c.name) === norm(r.fileName));
        return { ...r, column: match?.name ?? "", include: !!match };
      }),
    );
  }, [existingTable, createNew]);

  const canRun =
    !!preview &&
    !!tableName.trim() &&
    rows.some((r) => r.include && r.column.trim()) &&
    (createNew || !!existingTable);

  const run = async () => {
    if (!preview) return;
    setStep("running");
    setError(null);
    const mappings: ImportMapping[] = rows
      .filter((r) => r.include && r.column.trim())
      .map((r) => ({
        fileIndex: r.fileIndex,
        column: r.column.trim(),
        dataType: createNew ? r.dataType : undefined,
      }));
    try {
      const res = await api.importRun({
        profileId: target.profileId,
        jobId,
        path,
        delimiter,
        hasHeader: hasHeader ?? true,
        schema,
        table: tableName.trim(),
        createTable: createNew,
        mappings,
        emptyAsNull,
        batchSize: 200,
      });
      setResult(
        `Imported ${res.rowsImported.toLocaleString()} rows in ${(res.elapsedMs / 1000).toFixed(1)}s`,
      );
      setStep("done");
      void loadCatalog(target.profileId, true);
    } catch (e) {
      setError(errMessage(e));
      setStep("mapping");
    }
  };

  return (
    <div className="modal-backdrop" onClick={step === "running" ? undefined : closeModals}>
      <div className="modal" style={{ width: 760 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>
            Import file {profile ? `→ ${profile.name}` : ""}
          </span>
          {step !== "running" && (
            <button className="ghost small" onClick={closeModals}>
              ✕
            </button>
          )}
        </div>

        <div className="modal-body">
          {(step === "file" || step === "mapping") && (
            <>
              <div className="form-grid">
                <label>File</label>
                <div className="form-row-inline">
                  <input
                    style={{ flex: 1 }}
                    value={path}
                    onChange={(e) => setPath(e.target.value)}
                    onBlur={() => path && void loadPreview(path, delimiter, hasHeader)}
                    placeholder="/path/to/data.csv"
                  />
                  <button onClick={() => void pickFile()}>Browse…</button>
                </div>
                {preview && (
                  <>
                    <label>Format</label>
                    <div className="form-row-inline">
                      <select
                        value={delimiter}
                        onChange={(e) => {
                          setDelimiter(e.target.value);
                          void loadPreview(path, e.target.value, hasHeader);
                        }}
                      >
                        <option value=",">comma (,)</option>
                        <option value=";">semicolon (;)</option>
                        <option value={"\t"}>tab</option>
                        <option value="|">pipe (|)</option>
                      </select>
                      <label>
                        <input
                          type="checkbox"
                          checked={hasHeader ?? true}
                          onChange={(e) => {
                            setHasHeader(e.target.checked);
                            void loadPreview(path, delimiter, e.target.checked);
                          }}
                        />{" "}
                        first row is header
                      </label>
                      <label>
                        <input
                          type="checkbox"
                          checked={emptyAsNull}
                          onChange={(e) => setEmptyAsNull(e.target.checked)}
                        />{" "}
                        empty → NULL
                      </label>
                      <span className="hint">
                        {(preview.totalBytes / 1024 / 1024).toFixed(1)} MB
                      </span>
                    </div>
                    <label>Target</label>
                    <div className="form-row-inline">
                      {catalog && catalog.schemas.length > 1 && (
                        <select value={schema} onChange={(e) => setSchema(e.target.value)}>
                          {catalog.schemas.map((s) => (
                            <option key={s.name} value={s.name}>
                              {s.name}
                            </option>
                          ))}
                        </select>
                      )}
                      <label>
                        <input
                          type="radio"
                          checked={createNew}
                          onChange={() => setCreateNew(true)}
                        />{" "}
                        new table
                      </label>
                      <label>
                        <input
                          type="radio"
                          checked={!createNew}
                          onChange={() => setCreateNew(false)}
                        />{" "}
                        existing table
                      </label>
                      {createNew ? (
                        <input
                          style={{ flex: 1 }}
                          value={tableName}
                          onChange={(e) => setTableName(e.target.value)}
                          placeholder="new_table_name"
                        />
                      ) : (
                        <select
                          style={{ flex: 1 }}
                          value={tableName}
                          onChange={(e) => setTableName(e.target.value)}
                        >
                          <option value="">— pick table —</option>
                          {catalog?.schemas
                            .find((s) => s.name === schema)
                            ?.tables.filter((t) => t.kind === "table")
                            .map((t) => (
                              <option key={t.name} value={t.name}>
                                {t.name}
                              </option>
                            ))}
                        </select>
                      )}
                    </div>
                  </>
                )}
              </div>

              {loadingPreview && (
                <div className="placeholder" style={{ minHeight: 60 }}>
                  <span className="spinner" />
                </div>
              )}

              {preview && (
                <>
                  <h4 style={{ margin: "14px 0 6px" }}>Column mapping</h4>
                  <table className="map-table">
                    <thead>
                      <tr>
                        <th style={{ width: 30 }} />
                        <th>File column</th>
                        <th>→ Table column</th>
                        <th style={{ width: 140 }}>{createNew ? "Type" : "Column type"}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {rows.map((r, i) => (
                        <tr key={i}>
                          <td>
                            <input
                              type="checkbox"
                              checked={r.include}
                              onChange={(e) =>
                                setRows((rs) =>
                                  rs.map((x, k) =>
                                    k === i ? { ...x, include: e.target.checked } : x,
                                  ),
                                )
                              }
                            />
                          </td>
                          <td>{r.fileName}</td>
                          <td>
                            {createNew ? (
                              <input
                                value={r.column}
                                onChange={(e) =>
                                  setRows((rs) =>
                                    rs.map((x, k) =>
                                      k === i ? { ...x, column: e.target.value } : x,
                                    ),
                                  )
                                }
                              />
                            ) : (
                              <select
                                value={r.column}
                                onChange={(e) =>
                                  setRows((rs) =>
                                    rs.map((x, k) =>
                                      k === i ? { ...x, column: e.target.value } : x,
                                    ),
                                  )
                                }
                              >
                                <option value="">— skip —</option>
                                {existingTable?.columns.map((c) => (
                                  <option key={c.name} value={c.name}>
                                    {c.name}
                                  </option>
                                ))}
                              </select>
                            )}
                          </td>
                          <td>
                            {createNew ? (
                              <select
                                value={r.dataType}
                                onChange={(e) =>
                                  setRows((rs) =>
                                    rs.map((x, k) =>
                                      k === i ? { ...x, dataType: e.target.value } : x,
                                    ),
                                  )
                                }
                              >
                                <option value="bigint">bigint</option>
                                <option value="double">double</option>
                                <option value="boolean">boolean</option>
                                <option value="text">text</option>
                              </select>
                            ) : (
                              <span className="hint">
                                {existingTable?.columns.find((c) => c.name === r.column)
                                  ?.dataType ?? ""}
                              </span>
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>

                  <h4 style={{ margin: "14px 0 2px" }}>Preview</h4>
                  <div style={{ overflow: "auto", maxHeight: 170 }}>
                    <table className="preview-table">
                      <thead>
                        <tr>
                          {preview.columns.map((c, i) => (
                            <th key={i}>{c}</th>
                          ))}
                        </tr>
                      </thead>
                      <tbody>
                        {preview.rows.slice(0, 8).map((r, i) => (
                          <tr key={i}>
                            {preview.columns.map((_, k) => (
                              <td key={k}>{r[k] ?? ""}</td>
                            ))}
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </>
              )}
              {error && (
                <p className="msg-err" style={{ marginTop: 10 }}>
                  {error}
                </p>
              )}
            </>
          )}

          {step === "running" && (
            <div style={{ padding: "20px 6px" }}>
              <p style={{ marginBottom: 10 }}>
                Importing… {progress.rows.toLocaleString()} rows
                {progress.total > 0 &&
                  ` · ${((progress.bytes / progress.total) * 100).toFixed(0)}%`}
              </p>
              <div className="progress-outer">
                <div
                  className="progress-inner"
                  style={{
                    width:
                      progress.total > 0
                        ? `${Math.min(100, (progress.bytes / progress.total) * 100)}%`
                        : "30%",
                  }}
                />
              </div>
              <div style={{ marginTop: 14 }}>
                <button className="danger" onClick={() => void api.jobCancel(jobId)}>
                  Cancel import
                </button>
              </div>
            </div>
          )}

          {step === "done" && (
            <div style={{ padding: "20px 6px" }}>
              <p className="msg-ok">{result}</p>
            </div>
          )}
        </div>

        <div className="modal-footer">
          <span className="sep" />
          {step === "done" ? (
            <button className="primary" onClick={closeModals}>
              Close
            </button>
          ) : step !== "running" ? (
            <>
              <button onClick={closeModals}>Cancel</button>
              <button className="primary" disabled={!canRun} onClick={() => void run()}>
                Import
              </button>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function sanitizeIdent(s: string): string {
  const cleaned = s
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .replace(/^(\d)/, "_$1");
  return cleaned || "column";
}
