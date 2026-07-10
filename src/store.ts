import { create } from "zustand";
import {
  api,
  Catalog,
  ConnectionProfile,
  EnvStatus,
  RunError,
  StatementOutcome,
  errMessage,
  uid,
} from "./api";

export interface TabState {
  id: string;
  title: string;
  profileId: string | null;
  sql: string;
  running: boolean;
  results: StatementOutcome[];
  error: RunError | null;
  totalElapsedMs: number;
  activeResult: number;
  maxRows: number;
  /** bump to force the editor to reload `sql` from the store */
  sqlEpoch: number;
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

  tabs: TabState[];
  activeTabId: string;

  // modals
  connectionModal: { open: boolean; editing: ConnectionProfile | null; presetJar?: string };
  ddlModal: DdlTarget | null;
  importModal: ImportTarget | null;
  exportModal: ExportTarget | null;
  driversModal: boolean;
  toast: string | null;

  init(): Promise<void>;
  loadProfiles(): Promise<void>;
  loadCatalog(profileId: string, refresh?: boolean): Promise<void>;
  toggleExpanded(profileId: string): void;

  addTab(profileId?: string | null, sql?: string, autorun?: boolean): string;
  closeTab(id: string): void;
  setActiveTab(id: string): void;
  setTabSql(id: string, sql: string): void;
  setTabProfile(id: string, profileId: string | null): void;
  setTabLimit(id: string, maxRows: number): void;
  setActiveResult(id: string, idx: number): void;
  runQuery(tabId: string, sqlOverride?: string): Promise<void>;
  cancelQuery(tabId: string): void;

  openConnectionModal(editing?: ConnectionProfile | null, presetJar?: string): void;
  closeConnectionModal(): void;
  openDdl(t: DdlTarget): void;
  openImport(t: ImportTarget): void;
  openExport(t: ExportTarget): void;
  setDriversModal(open: boolean): void;
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
    sql,
    running: false,
    results: [],
    error: null,
    totalElapsedMs: 0,
    activeResult: 0,
    maxRows: 1_000,
    sqlEpoch: 0,
  };
}

export const useStore = create<Store>((set, get) => {
  const first = newTab(null);
  return {
    profiles: [],
    catalogs: {},
    catalogLoading: {},
    catalogErrors: {},
    expandedProfiles: {},
    env: null,

    tabs: [first],
    activeTabId: first.id,

    connectionModal: { open: false, editing: null },
    ddlModal: null,
    importModal: null,
    exportModal: null,
    driversModal: false,
    toast: null,

    async init() {
      await get().loadProfiles();
      try {
        set({ env: await api.envStatus() });
      } catch {
        // env status is informational only
      }
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
      if (autorun && profileId) {
        void get().runQuery(t.id);
      }
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

      const remaining = s.tabs.filter((t) => t.id !== id);
      const tabs = remaining.length ? remaining : [newTab(tab.profileId)];
      const activeTabId =
        s.activeTabId === id ? tabs[Math.max(0, s.tabs.findIndex((t) => t.id === id) - 1)].id : s.activeTabId;
      set({ tabs, activeTabId });
    },

    setActiveTab(id) {
      set({ activeTabId: id });
    },

    setTabSql(id, sql) {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, sql } : t)),
      }));
    },

    setTabProfile(id, profileId) {
      const tab = get().tabs.find((t) => t.id === id);
      if (tab?.profileId && tab.profileId !== profileId) {
        void api.sessionClose(tab.profileId, tab.id);
      }
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, profileId } : t)),
      }));
      if (profileId && !get().catalogs[profileId]) void get().loadCatalog(profileId);
    },

    setTabLimit(id, maxRows) {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, maxRows } : t)),
      }));
    },

    setActiveResult(id, idx) {
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === id ? { ...t, activeResult: idx } : t)),
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

      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === tabId ? { ...t, running: true, error: null } : t,
        ),
      }));
      try {
        const res = await api.queryRun(tab.profileId, tab.id, sql, tab.maxRows);
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
                }
              : t,
          ),
        }));
      } catch (e) {
        set((s) => ({
          tabs: s.tabs.map((t) =>
            t.id === tabId
              ? {
                  ...t,
                  running: false,
                  error: { message: errMessage(e), kind: "internal", statementIndex: 0 },
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
    closeModals() {
      set({ ddlModal: null, importModal: null, exportModal: null, driversModal: false });
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
