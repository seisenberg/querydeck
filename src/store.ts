import { create } from "zustand";
import { splitStatements } from "./sqlsplit";
import {
  api,
  Catalog,
  ConnectionProfile,
  ConsoleFile,
  EnvStatus,
  RunError,
  StatementOutcome,
  errMessage,
  uid,
} from "./api";

export interface LogEntry {
  id: string;
  ts: string;
  sql: string;
  status: "running" | "rows" | "affected" | "ok" | "error";
  rowCount?: number;
  affected?: number | null;
  elapsedMs?: number;
  error?: string;
}

export interface TabState {
  id: string;
  title: string;
  profileId: string | null;
  /** Per-tab database override (same connection, different DB). */
  database: string | null;
  sql: string;
  running: boolean;
  results: StatementOutcome[];
  error: RunError | null;
  totalElapsedMs: number;
  activeResult: number;
  maxRows: number;
  /** bump to force the editor to reload `sql` from the store */
  sqlEpoch: number;
  /** per-console execution log, newest first */
  log: LogEntry[];
  bottomTab: "results" | "log" | "export";
}

export interface DdlTarget {
  profileId: string;
  schema: string;
  table: string;
}

export interface ImportTarget {
  profileId: string;
  schema?: string;
  table?: string;
}

export interface ExportTarget {
  profileId: string;
  sql: string;
  suggestedName: string;
}

export const ROW_LIMITS = [100, 500, 1_000, 10_000, 100_000, 0] as const;

export function limitLabel(l: number): string {
  return l === 0 ? "No limit" : l.toLocaleString();
}

interface Store {
  profiles: ConnectionProfile[];
  catalogs: Record<string, Catalog>;
  catalogLoading: Record<string, boolean>;
  catalogErrors: Record<string, string>;
  expandedProfiles: Record<string, boolean>;
  env: EnvStatus | null;
  /** Databases available per profile (for the per-tab database switcher). */
  databasesByProfile: Record<string, string[]>;
  /** Open session counts per profile (for connection-state icons). */
  sessions: Record<string, number>;
  /** Saved (closed) consoles per profile, from the local filesystem. */
  closedConsoles: Record<string, ConsoleFile[]>;

  tabs: TabState[];
  activeTabId: string | null;

  // modals
  connectionModal: { open: boolean; editing: ConnectionProfile | null; presetJar?: string };
  ddlModal: DdlTarget | null;
  importModal: ImportTarget | null;
  exportModal: ExportTarget | null;
  driversModal: boolean;
  historyModal: boolean;
  tunnelsModal: boolean;
  connectionPicker: boolean;
  toast: string | null;

  init(): Promise<void>;
  loadProfiles(): Promise<void>;
  loadCatalog(profileId: string, refresh?: boolean): Promise<void>;
  toggleExpanded(profileId: string): void;
  loadDatabases(profileId: string): Promise<void>;
  refreshSessions(): Promise<void>;
  loadClosedConsoles(): Promise<void>;
  disconnectProfile(profileId: string): Promise<void>;

  addTab(profileId?: string | null, sql?: string, autorun?: boolean): string;
  closeTab(id: string): void;
  moveTab(id: string, toIndex: number): void;
  reopenConsole(file: ConsoleFile): void;
  deleteConsole(profileId: string, consoleId: string): Promise<void>;
  setActiveTab(id: string): void;
  setTabSql(id: string, sql: string): void;
  setTabProfile(id: string, profileId: string | null): void;
  setTabDatabase(id: string, database: string | null): void;
  setTabLimit(id: string, maxRows: number): void;
  setActiveResult(id: string, idx: number): void;
  setBottomTab(id: string, tab: "results" | "log" | "export"): void;
  runQuery(tabId: string, sqlOverride?: string): Promise<void>;
  cancelQuery(tabId: string): void;

  openConnectionModal(editing?: ConnectionProfile | null, presetJar?: string): void;
  closeConnectionModal(): void;
  openDdl(t: DdlTarget): void;
  openImport(t: ImportTarget): void;
  openExport(t: ExportTarget): void;
  setDriversModal(open: boolean): void;
  setHistoryModal(open: boolean): void;
  setTunnelsModal(open: boolean): void;
  setConnectionPicker(open: boolean): void;
  closeModals(): void;
  showToast(msg: string): void;
}

let tabCounter = 0;

function newTab(profileId: string | null, sql = ""): TabState {
  tabCounter += 1;
  return {
    id: uid(),
    title: `console ${tabCounter}`,
    profileId,
    database: null,
    sql,
    running: false,
    results: [],
    error: null,
    totalElapsedMs: 0,
    activeResult: 0,
    maxRows: 1_000,
    sqlEpoch: 0,
    log: [],
    bottomTab: "results",
  };
}

const MAX_LOG_ENTRIES = 300;

/** Fields worth restoring across app restarts — everything runtime
 * (results, log, running flags) starts fresh. */
interface PersistedTab {
  id: string;
  title: string;
  profileId: string | null;
  database: string | null;
  sql: string;
  maxRows: number;
}

interface PersistedSession {
  tabs: PersistedTab[];
  activeTabId: string | null;
}

let persistTimer: number | undefined;
/** Debounced write of the open-console layout, so quitting and relaunching
 * brings back the same set of consoles. */
function schedulePersist(get: () => { tabs: TabState[]; activeTabId: string | null }) {
  window.clearTimeout(persistTimer);
  persistTimer = window.setTimeout(() => {
    const s = get();
    const snapshot: PersistedSession = {
      tabs: s.tabs.map((t) => ({
        id: t.id,
        title: t.title,
        profileId: t.profileId,
        database: t.database,
        sql: t.sql,
        maxRows: t.maxRows,
      })),
      activeTabId: s.activeTabId,
    };
    void api.sessionStateSave(JSON.stringify(snapshot)).catch(() => undefined);
  }, 400);
}

function nowTime(): string {
  return new Date().toLocaleTimeString([], { hour12: false });
}

function logPreview(sql: string): string {
  const flat = sql.replace(/\s+/g, " ").trim();
  return flat.length > 300 ? flat.slice(0, 300) + "…" : flat;
}

export const useStore = create<Store>((set, get) => {
  return {
    profiles: [],
    catalogs: {},
    catalogLoading: {},
    catalogErrors: {},
    expandedProfiles: {},
    env: null,
    databasesByProfile: {},
    sessions: {},
    closedConsoles: {},

    tabs: [],
    activeTabId: null,

    connectionModal: { open: false, editing: null },
    ddlModal: null,
    importModal: null,
    exportModal: null,
    driversModal: false,
    historyModal: false,
    tunnelsModal: false,
    connectionPicker: false,
    toast: null,

    async init() {
      await get().loadProfiles();

      // Restore the console layout from the previous run (empty stays empty).
      try {
        const raw = await api.sessionStateLoad();
        if (raw) {
          const saved = JSON.parse(raw) as PersistedSession;
          const profileIds = new Set(get().profiles.map((p) => p.id));
          const tabs = (saved.tabs ?? [])
            .filter((t) => !t.profileId || profileIds.has(t.profileId))
            .map((t) => ({
              ...newTab(t.profileId ?? null, t.sql ?? ""),
              id: t.id,
              title: t.title,
              database: t.database ?? null,
              maxRows: t.maxRows ?? 1_000,
            }));
          // Continue numbering after the restored consoles.
          const maxN = tabs.reduce((m, t) => {
            const n = /^console (\d+)$/.exec(t.title);
            return n ? Math.max(m, Number(n[1])) : m;
          }, 0);
          tabCounter = Math.max(tabCounter, maxN);
          const activeTabId =
            tabs.find((t) => t.id === saved.activeTabId)?.id ?? tabs[tabs.length - 1]?.id ?? null;
          set({ tabs, activeTabId });
        }
      } catch {
        // a corrupt session file just means starting empty
      }

      void get().loadClosedConsoles();
      void get().refreshSessions();
      try {
        set({ env: await api.envStatus() });
      } catch {
        // env status is informational only
      }
    },

    async loadDatabases(profileId) {
      try {
        const dbs = await api.databasesList(profileId);
        set((s) => ({
          databasesByProfile: { ...s.databasesByProfile, [profileId]: dbs },
        }));
        void get().refreshSessions();
      } catch {
        // switching just stays unavailable for this profile
      }
    },

    async refreshSessions() {
      try {
        set({ sessions: await api.sessionsCount() });
      } catch {
        /* informational */
      }
    },

    async loadClosedConsoles() {
      try {
        const all = await api.consoleList();
        const grouped: Record<string, ConsoleFile[]> = {};
        for (const c of all) {
          if (!c.closed) continue;
          (grouped[c.profileId] ??= []).push(c);
        }
        set({ closedConsoles: grouped });
      } catch {
        /* tree section just stays empty */
      }
    },

    async disconnectProfile(profileId) {
      try {
        await api.profileDisconnect(profileId);
      } catch (e) {
        get().showToast(errMessage(e));
      }
      void get().refreshSessions();
    },

    async loadProfiles() {
      try {
        const profiles = await api.profilesList();
        set({ profiles });
      } catch (e) {
        get().showToast(errMessage(e));
      }
    },

    async loadCatalog(profileId, refresh = false) {
      if (get().catalogLoading[profileId]) return;
      set((s) => ({
        catalogLoading: { ...s.catalogLoading, [profileId]: true },
        catalogErrors: { ...s.catalogErrors, [profileId]: "" },
      }));
      try {
        const cat = await api.catalogGet(profileId, refresh);
        set((s) => ({ catalogs: { ...s.catalogs, [profileId]: cat } }));
      } catch (e) {
        set((s) => ({
          catalogErrors: { ...s.catalogErrors, [profileId]: errMessage(e) },
        }));
      } finally {
        set((s) => ({ catalogLoading: { ...s.catalogLoading, [profileId]: false } }));
      }
    },

    toggleExpanded(profileId) {
      const expanded = !get().expandedProfiles[profileId];
      set((s) => ({
        expandedProfiles: { ...s.expandedProfiles, [profileId]: expanded },
      }));
      if (expanded && !get().catalogs[profileId]) {
        void get().loadCatalog(profileId);
      }
    },

    addTab(profileId = null, sql = "", autorun = false) {
      const s0 = get();
      const active = s0.tabs.find((t) => t.id === s0.activeTabId);
      const t = newTab(profileId ?? active?.profileId ?? null, sql);
      set((s) => ({ tabs: [...s.tabs, t], activeTabId: t.id }));
      if (t.profileId) {
        void get().loadDatabases(t.profileId);
      }
      if (autorun && profileId) {
        void get().runQuery(t.id);
      }
      schedulePersist(get);
      return t.id;
    },

    closeTab(id) {
      const s = get();
      const tab = s.tabs.find((t) => t.id === id);
      if (!tab) return;
      const ids = tab.results
        .map((r) => r.resultId)
        .filter((r): r is number => r != null);
      if (ids.length) void api.resultsFree(ids);
      if (tab.profileId) void api.sessionClose(tab.profileId, tab.id);

      // Closing does not delete the console: persist it (sql + log) so it
      // stays under the connection in the tree and can be reopened.
      if (tab.profileId && (tab.sql.trim() || tab.log.length)) {
        void api
          .consoleSave(consoleFileFor(tab, true))
          .then(() => get().loadClosedConsoles())
          .catch(() => undefined);
      }

      // The tab list may go empty — the app shows an empty state and ⌘W
      // then closes the window instead of a console.
      const tabs = s.tabs.filter((t) => t.id !== id);
      const activeTabId =
        s.activeTabId === id
          ? (tabs[Math.max(0, s.tabs.findIndex((t) => t.id === id) - 1)]?.id ?? null)
          : s.activeTabId;
      set({ tabs, activeTabId });
      void get().refreshSessions();
      schedulePersist(get);
    },

    moveTab(id, toIndex) {
      set((s) => {
        const from = s.tabs.findIndex((t) => t.id === id);
        const to = Math.max(0, Math.min(s.tabs.length - 1, toIndex));
        if (from === -1 || from === to) return {};
        const tabs = [...s.tabs];
        const [moved] = tabs.splice(from, 1);
        tabs.splice(to, 0, moved);
        return { tabs };
      });
      schedulePersist(get);
    },

    reopenConsole(file) {
      const s = get();
      const already = s.tabs.find((t) => t.id === file.id);
      if (already) {
        set({ activeTabId: already.id });
        return;
      }
      tabCounter += 1;
      const t: TabState = {
        ...newTab(file.profileId, file.sql),
        id: file.id,
        title: file.title,
        database: file.database ?? null,
        log: (file.log as LogEntry[]) ?? [],
      };
      set((st) => ({
        tabs: [...st.tabs, t],
        activeTabId: t.id,
        closedConsoles: {
          ...st.closedConsoles,
          [file.profileId]: (st.closedConsoles[file.profileId] ?? []).filter(
            (c) => c.id !== file.id,
          ),
        },
      }));
      void api.consoleSave({ ...file, closed: false }).catch(() => undefined);
      void get().loadDatabases(file.profileId);
      schedulePersist(get);
    },

    async deleteConsole(profileId, consoleId) {
      try {
        await api.consoleDelete(profileId, consoleId);
      } catch (e) {
        get().showToast(errMessage(e));
      }
      void get().loadClosedConsoles();
    },

    setActiveTab(id) {
      set({ activeTabId: id });
      schedulePersist(get);
    },

    setTabSql(id, sql) {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, sql } : t)),
      }));
      schedulePersist(get);
    },

    setTabProfile(id, profileId) {
      const tab = get().tabs.find((t) => t.id === id);
      if (tab?.profileId && tab.profileId !== profileId) {
        void api.sessionClose(tab.profileId, tab.id);
      }
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, profileId, database: null } : t)),
      }));
      if (profileId && !get().catalogs[profileId]) void get().loadCatalog(profileId);
      if (profileId) void get().loadDatabases(profileId);
    },

    setTabDatabase(id, database) {
      const tab = get().tabs.find((t) => t.id === id);
      if (!tab || tab.database === database) return;
      // The session is bound to a database at connect time; close it so the
      // next run reconnects against the newly selected database.
      if (tab.profileId) void api.sessionClose(tab.profileId, tab.id);
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, database } : t)),
      }));
      void get().refreshSessions();
      schedulePersist(get);
    },

    setTabLimit(id, maxRows) {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, maxRows } : t)),
      }));
      schedulePersist(get);
    },

    setActiveResult(id, idx) {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, activeResult: idx } : t)),
      }));
    },

    setBottomTab(id, tab) {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, bottomTab: tab } : t)),
      }));
    },

    async runQuery(tabId, sqlOverride) {
      const tab = get().tabs.find((t) => t.id === tabId);
      if (!tab || tab.running) return;
      if (!tab.profileId) {
        get().showToast("Pick a connection for this tab first");
        return;
      }
      const sql = (sqlOverride ?? tab.sql).trim();
      if (!sql) return;

      // free previous results
      const oldIds = tab.results
        .map((r) => r.resultId)
        .filter((r): r is number => r != null);
      if (oldIds.length) void api.resultsFree(oldIds);

      // Log the run as started and flip the bottom pane to the Log tab.
      const runLogId = uid();
      const startedEntry: LogEntry = {
        id: runLogId,
        ts: nowTime(),
        sql: logPreview(sql),
        status: "running",
      };
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === tabId
            ? {
                ...t,
                running: true,
                error: null,
                bottomTab: "log",
                log: [startedEntry, ...t.log].slice(0, MAX_LOG_ENTRIES),
              }
            : t,
        ),
      }));

      const finishLog = (entries: LogEntry[]) => (t: TabState): LogEntry[] =>
        [...entries, ...t.log.filter((l) => l.id !== runLogId)].slice(0, MAX_LOG_ENTRIES);

      try {
        const res = await api.queryRun(tab.profileId, tab.id, sql, tab.maxRows, tab.database);
        // One finished entry per statement (newest first), plus the error if any.
        const entries: LogEntry[] = res.statements.map((st) => ({
          id: uid(),
          ts: nowTime(),
          sql: logPreview(st.sql),
          status: st.kind,
          rowCount: st.kind === "rows" ? st.rowCount : undefined,
          affected: st.affected,
          elapsedMs: st.elapsedMs,
        }));
        if (res.error) {
          entries.push({
            id: uid(),
            ts: nowTime(),
            sql: logPreview(splitStatementsForLog(sql, res.error.statementIndex)),
            status: "error",
            error: res.error.message,
          });
        }
        entries.reverse();
        set((s) => ({
          tabs: s.tabs.map((t) =>
            t.id === tabId
              ? {
                  ...t,
                  running: false,
                  results: res.statements,
                  error: res.error,
                  totalElapsedMs: res.totalElapsedMs,
                  activeResult: pickActiveResult(res.statements),
                  bottomTab: "results",
                  log: finishLog(entries)(t),
                }
              : t,
          ),
        }));
        void get().refreshSessions();
        const done = get().tabs.find((t) => t.id === tabId);
        if (done?.profileId) {
          void api.consoleSave(consoleFileFor(done, false)).catch(() => undefined);
          // Keep the database switcher fresh (e.g. after CREATE DATABASE).
          void get().loadDatabases(done.profileId);
        }
      } catch (e) {
        const message = errMessage(e);
        set((s) => ({
          tabs: s.tabs.map((t) =>
            t.id === tabId
              ? {
                  ...t,
                  running: false,
                  error: { message, kind: "internal", statementIndex: 0 },
                  bottomTab: "results",
                  log: finishLog([
                    {
                      id: uid(),
                      ts: nowTime(),
                      sql: logPreview(sql),
                      status: "error",
                      error: message,
                    },
                  ])(t),
                }
              : t,
          ),
        }));
      }
    },

    cancelQuery(tabId) {
      const tab = get().tabs.find((t) => t.id === tabId);
      if (tab?.profileId) void api.queryCancel(tab.profileId, tab.id);
    },

    openConnectionModal(editing = null, presetJar) {
      set({ connectionModal: { open: true, editing, presetJar } });
    },
    closeConnectionModal() {
      set({ connectionModal: { open: false, editing: null } });
    },
    openDdl(t) {
      set({ ddlModal: t });
    },
    openImport(t) {
      set({ importModal: t });
    },
    openExport(t) {
      set({ exportModal: t });
    },
    setDriversModal(open) {
      set({ driversModal: open });
    },
    setHistoryModal(open) {
      set({ historyModal: open });
    },
    setTunnelsModal(open) {
      set({ tunnelsModal: open });
    },
    setConnectionPicker(open) {
      set({ connectionPicker: open });
    },
    closeModals() {
      set({
        ddlModal: null,
        importModal: null,
        exportModal: null,
        driversModal: false,
        historyModal: false,
        tunnelsModal: false,
        connectionPicker: false,
      });
    },
    showToast(msg) {
      set({ toast: msg });
      setTimeout(() => {
        if (get().toast === msg) set({ toast: null });
      }, 5000);
    },
  } as Store;
});

function pickActiveResult(statements: StatementOutcome[]): number {
  const idx = statements.findIndex((s) => s.kind === "rows");
  return idx === -1 ? Math.max(0, statements.length - 1) : idx;
}

/** Text of the statement that failed (for the log entry). */
function splitStatementsForLog(script: string, statementIndex: number): string {
  return splitStatements(script)[statementIndex]?.sql ?? script;
}

/** Snapshot of a tab as a persistable console file. */
function consoleFileFor(tab: TabState, closed: boolean): ConsoleFile {
  return {
    id: tab.id,
    title: tab.title,
    profileId: tab.profileId ?? "",
    database: tab.database,
    sql: tab.sql,
    log: tab.log,
    updatedAt: new Date().toISOString().replace("T", " ").slice(0, 19),
    closed,
  };
}
