import { useMemo } from "react";
import type { SQLNamespace } from "@codemirror/lang-sql";
import { splitStatements } from "../sqlsplit";
import { ROW_LIMITS, TabState, limitLabel, useStore } from "../store";
import SqlEditor from "./SqlEditor";
import ResultsGrid from "./ResultsGrid";

interface Props {
  tab: TabState;
  active: boolean;
}

export default function QueryTab({ tab }: Props) {
  const profiles = useStore((s) => s.profiles);
  const catalogs = useStore((s) => s.catalogs);
  const setTabSql = useStore((s) => s.setTabSql);
  const setTabProfile = useStore((s) => s.setTabProfile);
  const setTabLimit = useStore((s) => s.setTabLimit);
  const setActiveResult = useStore((s) => s.setActiveResult);
  const runQuery = useStore((s) => s.runQuery);
  const cancelQuery = useStore((s) => s.cancelQuery);
  const openConnectionModal = useStore((s) => s.openConnectionModal);
  const openExport = useStore((s) => s.openExport);

  const profile = profiles.find((p) => p.id === tab.profileId) ?? null;
  const catalog = tab.profileId ? catalogs[tab.profileId] : undefined;

  const schemaNs: SQLNamespace | undefined = useMemo(() => {
    if (!catalog) return undefined;
    const ns: Record<string, Record<string, string[]>> = {};
    for (const s of catalog.schemas) {
      ns[s.name] = {};
      for (const t of s.tables) {
        ns[s.name][t.name] = t.columns.map((c) => c.name);
      }
    }
    // If there is a single schema, also expose tables at the top level.
    if (catalog.schemas.length === 1) {
      const only = catalog.schemas[0];
      const flat: SQLNamespace = {};
      for (const t of only.tables) flat[t.name] = t.columns.map((c) => c.name);
      return { ...flat, ...ns } as SQLNamespace;
    }
    return ns as SQLNamespace;
  }, [catalog]);

  const activeOutcome = tab.results[tab.activeResult];

  return (
    <>
      <div className="query-toolbar">
        <select
          value={tab.profileId ?? ""}
          onChange={(e) => setTabProfile(tab.id, e.target.value || null)}
          style={{ minWidth: 160 }}
          title="Connection for this tab"
        >
          <option value="">— no connection —</option>
          {profiles.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name} ({p.driver})
            </option>
          ))}
        </select>
        <button className="small ghost" onClick={() => openConnectionModal()} title="Add connection">
          +
        </button>

        {tab.running ? (
          <button className="danger" onClick={() => cancelQuery(tab.id)}>
            ■ Cancel
          </button>
        ) : (
          <>
            <button className="primary" onClick={() => void runQuery(tab.id)} title="Run everything (⌘⇧⏎)">
              ▶ Run
            </button>
          </>
        )}

        <label>
          Limit{" "}
          <select
            value={tab.maxRows}
            onChange={(e) => setTabLimit(tab.id, Number(e.target.value))}
            title="Automatic row limit applied to results"
          >
            {ROW_LIMITS.map((l) => (
              <option key={l} value={l}>
                {limitLabel(l)}
              </option>
            ))}
          </select>
        </label>

        <span className="sep" />

        {activeOutcome?.kind === "rows" && profile && (
          <button
            className="small"
            onClick={() =>
              openExport({
                profileId: profile.id,
                sql: fullStatementFor(tab, tab.activeResult),
                suggestedName: "result",
              })
            }
            title="Re-runs this statement and streams every row to a file (ignores the grid limit)"
          >
            Export…
          </button>
        )}
        <span className="hint">⌘⏎ statement · ⌘⇧⏎ all</span>
      </div>

      <SqlEditor
        initialSql={tab.sql}
        driver={profile?.driver ?? null}
        schema={schemaNs}
        onChange={(sql) => setTabSql(tab.id, sql)}
        onRunStatement={(sql) => void runQuery(tab.id, sql)}
        onRunAll={() => void runQuery(tab.id)}
      />

      <div className="results-area">
        {tab.error && (
          <div className="error-box">
            {tab.results.length > 0 && `statement ${tab.error.statementIndex + 1}: `}
            {tab.error.message}
          </div>
        )}

        {tab.results.length > 1 && (
          <div className="result-tabs">
            {tab.results.map((r, i) => (
              <button
                key={i}
                className={`rtab ${i === tab.activeResult ? "active" : ""}`}
                onClick={() => setActiveResult(tab.id, i)}
                title={r.sql}
              >
                {i + 1}.{" "}
                {r.kind === "rows"
                  ? `${r.rowCount.toLocaleString()} rows`
                  : r.kind === "affected"
                    ? `${r.affected} affected`
                    : "ok"}
              </button>
            ))}
          </div>
        )}

        {activeOutcome ? (
          activeOutcome.kind === "rows" ? (
            <>
              <ResultsGrid outcome={activeOutcome} />
              <div className="result-meta">
                <span>
                  {activeOutcome.rowCount.toLocaleString()} rows
                  {activeOutcome.truncated ? ` (stopped at limit ${limitLabel(tab.maxRows)})` : ""}
                </span>
                <span>{activeOutcome.elapsedMs} ms</span>
                <span title={activeOutcome.sql}>{activeOutcome.sql}</span>
              </div>
            </>
          ) : (
            <div className="placeholder">
              <div>
                {activeOutcome.kind === "affected"
                  ? `${activeOutcome.affected?.toLocaleString()} row(s) affected`
                  : "OK"}
              </div>
              <div className="hint">{activeOutcome.sql}</div>
            </div>
          )
        ) : (
          !tab.error && (
            <div className="placeholder">
              {tab.running ? (
                <>
                  <span className="spinner" /> running…
                </>
              ) : (
                <>
                  <div>Write SQL above, then ⌘⏎ to run the statement at the cursor.</div>
                  <div className="hint">Results appear here. Each tab has its own connection.</div>
                </>
              )}
            </div>
          )
        )}
      </div>
    </>
  );
}

/** Full text of the i-th statement in the tab's script (for export). */
function fullStatementFor(tab: TabState, idx: number): string {
  // The preview stored on the outcome may be truncated; re-split the script.
  const outcome = tab.results[idx];
  if (!outcome) return tab.sql;
  return outcome.sql.length < 120
    ? outcome.sql
    : (splitStatements(tab.sql)[idx]?.sql ?? outcome.sql);
}
