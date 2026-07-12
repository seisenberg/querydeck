import { useMemo, useState } from "react";
import { format as sqlFormat } from "sql-formatter";
import { DEFAULT_FORMATTER, FormatterConfig, useStore } from "../store";

const SAMPLE =
  "select o.id, o.total, c.name from orders o join customers c on c.id=o.customer_id where o.total>100 order by o.total desc;";

/** Edit → Formatter Settings…: how "Reformat Query" lays out SQL. Options
 * persist in the app config file, so they apply across restarts. */
export default function FormatterSettings() {
  const setFormatterModal = useStore((s) => s.setFormatterModal);
  const setFormatterConfig = useStore((s) => s.setFormatterConfig);
  const saved = useStore((s) => s.appConfig.formatter);
  const [cfg, setCfg] = useState<FormatterConfig>(saved ?? DEFAULT_FORMATTER);

  const preview = useMemo(() => {
    try {
      return sqlFormat(SAMPLE, {
        language: "sql",
        keywordCase: cfg.keywordCase,
        tabWidth: cfg.tabWidth,
        linesBetweenQueries: cfg.linesBetweenQueries,
        denseOperators: cfg.denseOperators,
      });
    } catch {
      return SAMPLE;
    }
  }, [cfg]);

  const patch = (p: Partial<FormatterConfig>) => setCfg((c) => ({ ...c, ...p }));

  return (
    <div className="modal-backdrop" onClick={() => setFormatterModal(false)}>
      <div className="modal" style={{ width: 560 }} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>SQL formatter settings</span>
          <button className="ghost small" onClick={() => setFormatterModal(false)}>
            ✕
          </button>
        </div>
        <div className="modal-body">
          <div className="form-grid">
            <label>Keyword case</label>
            <select
              value={cfg.keywordCase}
              onChange={(e) => patch({ keywordCase: e.target.value as FormatterConfig["keywordCase"] })}
            >
              <option value="upper">UPPERCASE</option>
              <option value="lower">lowercase</option>
              <option value="preserve">preserve as written</option>
            </select>

            <label>Indent width</label>
            <select
              value={cfg.tabWidth}
              onChange={(e) => patch({ tabWidth: Number(e.target.value) })}
            >
              {[2, 4, 8].map((n) => (
                <option key={n} value={n}>
                  {n} spaces
                </option>
              ))}
            </select>

            <label>Between statements</label>
            <select
              value={cfg.linesBetweenQueries}
              onChange={(e) => patch({ linesBetweenQueries: Number(e.target.value) })}
            >
              {[0, 1, 2, 3].map((n) => (
                <option key={n} value={n}>
                  {n} blank line{n === 1 ? "" : "s"}
                </option>
              ))}
            </select>

            <label>Operators</label>
            <label>
              <input
                type="checkbox"
                checked={cfg.denseOperators}
                onChange={(e) => patch({ denseOperators: e.target.checked })}
              />{" "}
              dense (no spaces around operators)
            </label>
          </div>

          <div className="hint" style={{ margin: "12px 0 4px" }}>
            Preview
          </div>
          <pre className="formatter-preview">{preview}</pre>
        </div>
        <div className="modal-footer">
          <button
            onClick={() => setCfg(DEFAULT_FORMATTER)}
            title="Reset to the default formatting options"
          >
            Reset
          </button>
          <span className="sep" />
          <button onClick={() => setFormatterModal(false)}>Cancel</button>
          <button
            className="primary"
            onClick={() => {
              setFormatterConfig(cfg);
              setFormatterModal(false);
            }}
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
