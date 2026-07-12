import { memo, useMemo, useRef } from "react";
import type { SQLNamespace } from "@codemirror/lang-sql";
import { splitStatements } from "../sqlsplit";
import { LogEntry, ROW_LIMITS, TabState, limitLabel, useStore } from "../store";
import SqlEditor from "./SqlEditor";
import ResultsGrid from "./ResultsGrid";
import ExportPane from "./ExportPane";
import { DbIcon, engineOf } from "./DbIcons";

interface Props {
  tab: TabState;
  active: boolean;
}

const NO_DBS: string[] = [];

/** Memoized: keystrokes update one tab's state, and only that pane (plus the
 * tab strip) should re-render — not every open console and its grid. */
export default memo(QueryTab);

function QueryTab({ tab }: Props) {
  const profiles = useStore((s) => s.profiles);
  // Narrow, per-connection selectors: refreshes for other connections must
  // not re-render this console.
  const catalog = useStore((s) => (tab.profileId ? s.catalogs[tab.profileId] : undefined));
  const openSessions = useStore((s) =>
    tab.profileId ? (s.sessions[tab.profileId] ?? 0) : 0,
  );
  const databases = useStore((s) =>
    tab.profileId ? s.databasesByProfile[tab.profileId] : undefined,
  );
  const editorFrac = useStore((s) => s.editorFrac);
  const setEditorFrac = useStore((s) => s.setEditorFrac);
  const setTabSql = useStore((s) => s.setTabSql);
  const setTabDatabase = useStore((s) => s.setTabDatabase);
  const setTabLimit = useStore((s) => s.setTabLimit);
  const setActiveResult = useStore((s) => s.setActiveResult);
  const setBottomTab = useStore((s) => s.setBottomTab);
  const runQuery = useStore((s) => s.runQuery);
  const cancelQuery = useStore((s) => s.cancelQuery);
  const setConnectionPicker = useStore((s) => s.setConnectionPicker);

  const splitRef = useRef<HTMLDivElement>(null);
  const splitRect = useRef<DOMRect | null>(null);

  const profile = profiles.find((p) => p.id === tab.profileId) ?? null;
  const connected = openSessions > 0;

  // Pointer capture keeps the drag alive no matter how the panes re-render
  // underneath (window-level mouse listeners proved flaky mid-drag).
  const onSplitPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || !splitRef.current) return;
    e.preventDefault();
    splitRect.current = splitRef.current.getBoundingClientRect();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const onSplitPointerMove = (e: React.PointerEvent) => {
    const rect = splitRect.current;
    if (!rect || !(e.buttons & 1)) return;
    setEditorFrac((e.clientY - rect.top) / rect.height);
  };
  const onSplitPointerUp = () => {
    splitRect.current = null;
  };

  const schemaNs: SQLNamespace | undefined = useMemo(() => {
    if (!catalog) return undefined;
    const ns: Record<string, Record<string, string[]>> = {};
    for (const s of catalog.schemas) {
      ns[s.name] = {};
      for (const t of s.tables) {
        ns[s.name][t.name] = t.columns.map((c) => c.name);
      }
    }
    if (catalog.schemas.length === 1) {
      const only = catalog.schemas[0];
      const flat: SQLNamespace = {};
      for (const t of only.tables) flat[t.name] = t.columns.map((c) => c.name);
      return { ...flat, ...ns } as SQLNamespace;
    }
    return ns as SQLNamespace;
  }, [catalog]);

  const activeOutcome = tab.results[tab.activeResult];
  const exportSql = activeOutcome?.kind === "rows" ? fullStatementFor(tab, tab.activeResult) : "";

  return (
    <>
      <div className="query-toolbar">
        {profile ? (
          <span className="conn-label" title={`${profile.name} (${profile.driver})`}>
            <DbIcon engine={engineOf(profile)} connected={connected} />
            <span>{profile.name}</span>
          </span>
        ) : (
          <button className="small" onClick={() => setConnectionPicker(true)}>
            Pick connection…
          </button>
        )}

        {profile && (databases ?? NO_DBS).length > 0 && (
          <label>
            DB{" "}
            <select
              value={tab.database ?? defaultDatabase(profile)}
              onChange={(e) => setTabDatabase(tab.id, e.target.value)}
              title="Switch database on this connection (reconnects this console)"
              style={{ maxWidth: 180 }}
            >
              {(databases ?? NO_DBS).map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
          </label>
        )}

        {tab.running ? (
          <button className="danger" onClick={() => cancelQuery(tab.id)}>
            ■ Cancel
          </button>
        ) : (
          <button className="primary" onClick={() => void runQuery(tab.id)} title="Run everything (⌘⇧⏎)">
            ▶ Run
          </button>
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
        <span className="hint">⌘⏎ statement · ⌘⇧⏎ all</span>
      </div>

      <div className="split-host" ref={splitRef}>
        <SqlEditor
          initialSql={tab.sql}
          sqlEpoch={tab.sqlEpoch}
          driver={profile?.driver ?? null}
          schema={schemaNs}
          heightPct={editorFrac}
          onChange={(sql) => setTabSql(tab.id, sql)}
          onRunStatement={(sql) => void runQuery(tab.id, sql)}
          onRunAll={() => void runQuery(tab.id)}
        />

        <div
          className="h-splitter"
          onPointerDown={onSplitPointerDown}
          onPointerMove={onSplitPointerMove}
          onPointerUp={onSplitPointerUp}
          title="Drag to resize"
        />

        <div className="results-area">
        <div className="bottom-tabs">
          <button
            className={`btab ${tab.bottomTab === "results" ? "active" : ""}`}
            onClick={() => setBottomTab(tab.id, "results")}
          >
            Results
          </button>
          <button
            className={`btab ${tab.bottomTab === "export" ? "active" : ""}`}
            onClick={() => setBottomTab(tab.id, "export")}
          >
            Export
          </button>
          <button
            className={`btab ${tab.bottomTab === "log" ? "active" : ""}`}
            onClick={() => setBottomTab(tab.id, "log")}
          >
            Log{tab.log.length > 0 ? ` (${tab.log.length})` : ""}
          </button>
        </div>

        {tab.bottomTab === "log" && <LogPane entries={tab.log} />}

        {tab.bottomTab === "export" &&
          (profile ? (
            <ExportPane
              profileId={profile.id}
              sql={exportSql || tab.sql}
              tableName="result"
              database={tab.database}
            />
          ) : (
            <div className="placeholder">
              <div>Pick a connection first.</div>
            </div>
          ))}

        {tab.bottomTab === "results" && tab.error && (
          <div className="error-box">
            {tab.results.length > 0 && `statement ${tab.error.statementIndex + 1}: `}
            {tab.error.message}
          </div>
        )}

        {tab.bottomTab === "results" && tab.results.length > 1 && (
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

        {tab.bottomTab === "results" && activeOutcome ? (
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
          tab.bottomTab === "results" &&
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
      </div>
    </>
  );
}

function defaultDatabase(profile: { params: unknown }): string {
  const p = profile.params as { database?: string };
  return p.database ?? "";
}

function LogPane({ entries }: { entries: LogEntry[] }) {
  if (entries.length === 0) {
    return (
      <div className="placeholder">
        <div>No queries run in this console yet.</div>
        <div className="hint">Each run is logged here with its timing and outcome.</div>
      </div>
    );
  }
  return (
    <div className="log-pane">
      {entries.map((e) => (
        <div key={e.id} className={`log-row ${e.status}`}>
          <span className="log-ts">{e.ts}</span>
          <span className={`log-status ${e.status}`}>
            {e.status === "running" && (
              <>
                <span className="spinner" /> running
              </>
            )}
            {e.status === "rows" && `${(e.rowCount ?? 0).toLocaleString()} rows`}
            {e.status === "affected" && `${(e.affected ?? 0).toLocaleString()} affected`}
            {e.status === "ok" && "ok"}
            {e.status === "error" && "error"}
          </span>
          {e.elapsedMs != null && <span className="log-elapsed">{e.elapsedMs} ms</span>}
          <span className="log-sql" title={e.error ? `${e.sql}\n\n${e.error}` : e.sql}>
            {e.sql}
          </span>
          {e.error && <span className="log-error">{e.error}</span>}
        </div>
      ))}
    </div>
  );
}

/** Full text of the i-th statement in the tab's script (for export). */
function fullStatementFor(tab: TabState, idx: number): string {
  const outcome = tab.results[idx];
  if (!outcome) return tab.sql;
  return outcome.sql.length < 120
    ? outcome.sql
    : (splitStatements(tab.sql)[idx]?.sql ?? outcome.sql);
}
