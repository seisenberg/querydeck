import { useEffect, useState } from "react";
import { api, errMessage } from "../api";
import { DdlTarget, useStore } from "../store";

export default function DdlModal({ target }: { target: DdlTarget }) {
  const closeModals = useStore((s) => s.closeModals);
  const showToast = useStore((s) => s.showToast);
  const [ddl, setDdl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    setDdl(null);
    setError(null);
    api
      .ddlGet(target.profileId, target.schema, target.table)
      .then((r) => alive && setDdl(r.ddl))
      .catch((e) => alive && setError(errMessage(e)));
    return () => {
      alive = false;
    };
  }, [target.profileId, target.schema, target.table]);

  return (
    <div className="modal-backdrop" onClick={closeModals}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>
            DDL — {target.schema !== "(default)" && target.schema !== "main" ? `${target.schema}.` : ""}
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
