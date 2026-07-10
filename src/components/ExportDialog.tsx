import { ExportTarget, useStore } from "../store";
import ExportPane from "./ExportPane";

/** Table export modal (from the schema tree); the per-console Export tab uses
 * ExportPane directly. */
export default function ExportDialog({ target }: { target: ExportTarget }) {
  const closeModals = useStore((s) => s.closeModals);

  return (
    <div className="modal-backdrop" onClick={closeModals}>
      <div className="modal" style={{ width: 640 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>Export — {target.suggestedName}</span>
          <button className="ghost small" onClick={closeModals}>
            ✕
          </button>
        </div>
        <div className="modal-body">
          <ExportPane
            profileId={target.profileId}
            sql={target.sql}
            tableName={target.suggestedName}
          />
        </div>
      </div>
    </div>
  );
}
