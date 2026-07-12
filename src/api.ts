import { invoke } from "@tauri-apps/api/core";

// ---- Types mirrored from the Rust backend ----

export type DriverKind = "sqlite" | "postgres" | "mysql" | "odbc" | "jdbc";

export interface SqliteParams {
  path: string;
}
export interface PostgresParams {
  host: string;
  port: number;
  user: string;
  password: string;
  database: string;
  ssl_mode: "disable" | "prefer" | "require";
  accept_invalid_certs: boolean;
}
export interface MysqlParams {
  host: string;
  port: number;
  user: string;
  password: string;
  database: string;
  use_ssl: boolean;
  accept_invalid_certs: boolean;
}
export interface OdbcParams {
  connection_string: string;
}
export interface JdbcParams {
  url: string;
  driver_class: string;
  jars: string[];
  user: string;
  password: string;
}

export type DriverParams =
  | SqliteParams
  | PostgresParams
  | MysqlParams
  | OdbcParams
  | JdbcParams;

export interface ConnectionProfile {
  id: string;
  name: string;
  driver: DriverKind;
  params: DriverParams;
  /** Optional SSH tunnel id that is brought up before connecting. */
  sshTunnel?: string | null;
}

export interface ColumnMeta {
  name: string;
  typeName: string;
}

export type CellValue = null | boolean | number | string;

export interface StatementOutcome {
  sql: string;
  kind: "rows" | "affected" | "ok";
  resultId: number | null;
  columns: ColumnMeta[] | null;
  rowCount: number;
  truncated: boolean;
  affected: number | null;
  elapsedMs: number;
}

export interface RunError {
  message: string;
  kind: string;
  statementIndex: number;
}

export interface RunResponse {
  statements: StatementOutcome[];
  error: RunError | null;
  totalElapsedMs: number;
}

export interface ColumnInfo {
  name: string;
  dataType: string;
  nullable: boolean;
  default?: string;
}

export interface TableInfo {
  name: string;
  kind: string;
  columns: ColumnInfo[];
  pk: string[];
  indexes?: string[];
  foreign_keys?: string[];
}

export interface SchemaInfo {
  name: string;
  tables: TableInfo[];
}

/** Non-table schema object for the tree's "Other objects" section. */
export interface DbObject {
  schema?: string | null;
  name: string;
  /** "function" | "procedure" | "sequence" | "trigger" | "role" */
  kind: string;
  detail?: string | null;
}

export interface Catalog {
  schemas: SchemaInfo[];
}

export interface ImportPreview {
  columns: string[];
  rows: string[][];
  delimiter: string;
  hasHeader: boolean;
  totalBytes: number;
  inferredTypes: string[];
}

export interface ImportMapping {
  fileIndex: number;
  column: string;
  dataType?: string;
}

export interface ImportRequest {
  profileId: string;
  jobId: string;
  path: string;
  delimiter: string;
  hasHeader: boolean;
  schema: string;
  table: string;
  createTable: boolean;
  mappings: ImportMapping[];
  emptyAsNull: boolean;
  batchSize: number;
}

export interface ImportResult {
  rowsImported: number;
  elapsedMs: number;
}

export interface ExportRequest {
  profileId: string;
  jobId: string;
  sql: string;
  format: "csv" | "tsv" | "json" | "sql" | "xlsx";
  path: string;
  includeHeader: boolean;
  limit: number;
  tableName: string;
  database?: string | null;
  includeCreate?: boolean;
  insertStyle?: "single" | "multiline";
  batchLimit?: number;
}

export interface ConsoleFile {
  id: string;
  title: string;
  profileId: string;
  database?: string | null;
  sql: string;
  log: unknown[];
  updatedAt: string;
  closed: boolean;
}

export interface TunnelConfig {
  id: string;
  name: string;
  sshHost: string;
  sshPort: number;
  sshUser: string;
  keyPath?: string | null;
  localPort: number;
  remoteHost: string;
  remotePort: number;
}

export interface TunnelStatus {
  id: string;
  running: boolean;
}

export interface OdbcInventory {
  drivers: string[];
  dsns: [string, string][];
}

export interface ExportResult {
  rowsExported: number;
  elapsedMs: number;
}

export interface DriverPackage {
  id: string;
  title: string;
  version: string;
  url: string;
  filename: string;
  driverClass: string;
  urlTemplate: string;
  defaultPort: number;
  notes: string;
  downloaded: boolean;
  path: string | null;
}

export interface HistoryEntry {
  ts: string;
  profileId: string;
  profileName: string;
  sql: string;
  kind: "rows" | "affected" | "ok" | "error";
  rowCount?: number;
  affected?: number;
  elapsedMs: number;
  error?: string;
}

export interface EnvStatus {
  odbcAvailable: boolean;
  odbcDetail: string;
  jvmAvailable: boolean;
  jvmDetail: string;
}

export interface AppError {
  kind: string;
  message: string;
}

export function errMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) {
    return String((e as AppError).message);
  }
  return String(e);
}

// ---- Command wrappers ----

export const api = {
  profilesList: () => invoke<ConnectionProfile[]>("profiles_list"),
  profileSave: (profile: ConnectionProfile) =>
    invoke<void>("profile_save", { profile }),
  profileDelete: (profileId: string) =>
    invoke<void>("profile_delete", { profileId }),
  profilesReorder: (ids: string[]) => invoke<void>("profiles_reorder", { ids }),
  connectionTest: (profile: ConnectionProfile) =>
    invoke<string>("connection_test", { profile }),

  queryRun: (
    profileId: string,
    tabId: string,
    sql: string,
    maxRows: number,
    database?: string | null,
  ) =>
    invoke<RunResponse>("query_run", {
      profileId,
      tabId,
      sql,
      maxRows,
      database: database || null,
    }),
  queryCancel: (profileId: string, tabId: string) =>
    invoke<void>("query_cancel", { profileId, tabId }),
  pageFetch: (resultId: number, offset: number, limit: number) =>
    invoke<{ rows: CellValue[][] }>("page_fetch", { resultId, offset, limit }),
  resultsFree: (resultIds: number[]) =>
    invoke<void>("results_free", { resultIds }),
  sessionClose: (profileId: string, tabId: string) =>
    invoke<void>("session_close", { profileId, tabId }),

  catalogGet: (profileId: string, refresh: boolean) =>
    invoke<Catalog>("catalog_get", { profileId, refresh }),
  objectsList: (profileId: string, database?: string | null) =>
    invoke<DbObject[]>("objects_list", { profileId, database: database ?? null }),
  objectDdl: (
    profileId: string,
    kind: string,
    schema: string | null,
    name: string,
    detail: string | null,
    database?: string | null,
  ) =>
    invoke<string>("object_ddl", {
      profileId,
      kind,
      schema,
      name,
      detail,
      database: database ?? null,
    }),

  ddlGet: (profileId: string, schema: string, table: string) =>
    invoke<{ ddl: string; native: boolean }>("ddl_get", { profileId, schema, table }),

  importPreview: (path: string, delimiter?: string, hasHeader?: boolean) =>
    invoke<ImportPreview>("import_preview", {
      path,
      delimiter: delimiter ?? null,
      hasHeader: hasHeader ?? null,
    }),
  importRun: (req: ImportRequest) => invoke<ImportResult>("import_run", { req }),
  exportRun: (req: ExportRequest) => invoke<ExportResult>("export_run", { req }),
  jobCancel: (jobId: string) => invoke<void>("job_cancel", { jobId }),

  driversRegistry: () => invoke<DriverPackage[]>("drivers_registry"),
  driverDownload: (id: string) => invoke<string>("driver_download", { id }),
  envStatus: () => invoke<EnvStatus>("env_status"),

  historyList: (limit?: number, filter?: string, profileId?: string) =>
    invoke<HistoryEntry[]>("history_list", {
      limit: limit ?? 500,
      filter: filter || null,
      profileId: profileId || null,
    }),
  historyClear: () => invoke<void>("history_clear"),

  profileDisconnect: (profileId: string) =>
    invoke<void>("profile_disconnect", { profileId }),
  sessionsCount: () => invoke<Record<string, number>>("sessions_count"),
  databasesList: (profileId: string) =>
    invoke<string[]>("databases_list", { profileId }),

  sessionStateSave: (json: string) => invoke<void>("session_state_save", { json }),
  sessionStateLoad: () => invoke<string | null>("session_state_load"),

  configSave: (json: string) => invoke<void>("config_save", { json }),
  configLoad: () => invoke<string | null>("config_load"),

  consoleSave: (console: ConsoleFile) => invoke<void>("console_save", { console }),
  consoleList: (profileId?: string) =>
    invoke<ConsoleFile[]>("console_list", { profileId: profileId || null }),
  consoleDelete: (profileId: string, consoleId: string) =>
    invoke<void>("console_delete", { profileId, consoleId }),

  tunnelsList: () => invoke<TunnelConfig[]>("tunnels_list"),
  tunnelSave: (tunnel: TunnelConfig) => invoke<void>("tunnel_save", { tunnel }),
  tunnelDelete: (id: string) => invoke<void>("tunnel_delete", { id }),
  tunnelStart: (id: string) => invoke<void>("tunnel_start", { id }),
  tunnelStop: (id: string) => invoke<void>("tunnel_stop", { id }),
  tunnelsStatus: () => invoke<TunnelStatus[]>("tunnels_status"),

  odbcInventory: () => invoke<OdbcInventory>("odbc_inventory"),
  driversDir: () => invoke<string>("drivers_dir"),
};

export function uid(): string {
  return Math.random().toString(36).slice(2, 10) + Date.now().toString(36);
}
