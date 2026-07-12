import { useEffect, useState } from "react";
import { api, errMessage } from "../api";
import { DdlTarget, useStore } from "../store";

export default function DdlModal({ target }: { target: DdlTarget }) {
  const closeModals = useStore((s) => s.closeModals);
  const showToast = useStore((s) => s.showToast);
  const addTab = useStore((s) => s.addTab);
  const [ddl, setDdl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const isTable = !target.kind || target.kind === "table";

  useEffect(() => {
    let alive = true;
    setDdl(null);
    setError(null);
    const req = isTable
      ? api.ddlGet(target.profileId, target.schema, target.table).then((r) => r.ddl)
      : api.objectDdl(
          target.profileId,
          target.kind ?? "table",
          target.schema || null,
          target.table,
          target.detail ?? null,
        );
    req
      .then((d) => alive && setDdl(d))
      .catch((e) => alive && setError(errMessage(e)));
    return () => {
      alive = false;
    };
  }, [target.profileId, target.schema, target.table, target.kind, target.detail, isTable]);

  return (
    <div className="modal-backdrop" onClick={closeModals}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>
            {isTable ? "DDL" : `DDL · ${target.kind}`} —{" "}
            {target.schema && target.schema !== "(default)" && target.schema !== "main"
              ? `${target.schema}.`
              : ""}
            {target.table}
          </span>
          <button className="ghost small" onClick={closeModals}>
            ✕
          </button>
        </div>
        <div className="modal-body">
          {error && <div className="msg-err">{error}</div>}
          {!error && ddl === null && (
            <div className="placeholder" style={{ minHeight: 120 }}>
              <span className="spinner" />
            </div>
          )}
          {ddl !== null && <pre className="ddl-view">{ddl}</pre>}
        </div>
        <div className="modal-footer">
          <span className="sep" />
          <button
            onClick={() => {
              // Redefining the object starts from its current definition.
              if (ddl) {
                addTab(target.profileId, ddl);
                closeModals();
              }
            }}
            disabled={!ddl}
            title="Open this DDL in a new console tab to edit and re-run it"
          >
            Edit in console
          </button>
          <button
            onClick={() => {
              if (ddl) {
                void navigator.clipboard.writeText(ddl);
                showToast("DDL copied to clipboard");
              }
            }}
            disabled={!ddl}
          >
            Copy
          </button>
          <button className="primary" onClick={closeModals}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
