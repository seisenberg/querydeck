mod catalog;
mod consoles;
mod ddl;
mod drivers;
mod error;
mod history;
mod objects;
mod registry;
mod secrets;
mod sqlsplit;
mod state;
mod transfer;
mod tunnels;
mod value;

/// Re-exports for integration tests.
pub mod testkit {
    pub use crate::catalog::*;
    pub use crate::ddl::generate_table_ddl;
    pub use crate::drivers::{
        connect, ClickhouseParams, ConnectionProfile, DbConnection, Dialect, DriverParams,
        RowEvent, SqliteParams,
    };
    pub use crate::error::ErrorKind;
    pub use crate::sqlsplit::split_statements;
    pub use crate::state::AppState;
    pub use crate::value::{ColumnMeta, DbValue};
}

use std::collections::HashMap;
use std::ops::ControlFlow;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::State;

use catalog::Catalog;
use drivers::{ConnectionProfile, RowEvent};
use error::{AppError, AppResult};
use state::AppState;
use value::{ColumnMeta, DbValue};

/// Catalog cache, keyed by profile id: in-memory for the session, mirrored
/// to disk so the tree renders instantly on the next launch. `refreshing`
/// guards duplicate background refreshes per profile per app run.
#[derive(Default)]
struct CatalogCache {
    mem: Mutex<HashMap<String, Catalog>>,
    refreshed: Mutex<std::collections::HashSet<String>>,
}

impl CatalogCache {
    fn dir() -> std::path::PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("QueryDeck")
            .join("catalog-cache")
    }

    fn disk_path(profile_id: &str) -> std::path::PathBuf {
        // Profile ids are uid()-generated alphanumerics; sanitize anyway.
        let safe: String = profile_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        Self::dir().join(format!("{safe}.json"))
    }

    fn disk_load(profile_id: &str) -> Option<Catalog> {
        let bytes = std::fs::read(Self::disk_path(profile_id)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn disk_store(profile_id: &str, cat: &Catalog) {
        let _ = std::fs::create_dir_all(Self::dir());
        if let Ok(json) = serde_json::to_vec(cat) {
            let _ = std::fs::write(Self::disk_path(profile_id), json);
        }
    }

    fn disk_clear(profile_id: &str) {
        let _ = std::fs::remove_file(Self::disk_path(profile_id));
    }

    fn put(&self, profile_id: &str, cat: &Catalog) {
        self.mem.lock().insert(profile_id.to_string(), cat.clone());
        Self::disk_store(profile_id, cat);
    }
}

// ---------------------------------------------------------------- activity --

/// Live work registry backing the statusbar activity icon: user queries and
/// background jobs (schema loads, imports/exports, cache refreshes).
#[derive(Default)]
struct ActivityRegistry {
    items: Mutex<HashMap<u64, ActivityItem>>,
    next: std::sync::atomic::AtomicU64,
}

#[derive(Clone, Serialize)]
struct ActivityItem {
    id: u64,
    /// "query" (user-initiated) | "background"
    kind: String,
    label: String,
    #[serde(rename = "profileId")]
    profile_id: String,
    #[serde(rename = "startedMs")]
    started_ms: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl ActivityRegistry {
    fn begin(&self, app: &tauri::AppHandle, kind: &str, profile_id: &str, label: &str) -> u64 {
        use tauri::Emitter;
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.items.lock().insert(
            id,
            ActivityItem {
                id,
                kind: kind.to_string(),
                label: label.to_string(),
                profile_id: profile_id.to_string(),
                started_ms: now_ms(),
            },
        );
        let _ = app.emit("activity:changed", ());
        id
    }

    fn end(&self, app: &tauri::AppHandle, id: u64) {
        use tauri::Emitter;
        self.items.lock().remove(&id);
        let _ = app.emit("activity:changed", ());
    }

    fn list(&self) -> Vec<ActivityItem> {
        let mut v: Vec<ActivityItem> = self.items.lock().values().cloned().collect();
        v.sort_by_key(|i| i.started_ms);
        v
    }
}

#[tauri::command]
fn activity_list(activity: State<'_, Arc<ActivityRegistry>>) -> Vec<ActivityItem> {
    activity.list()
}

/// Kill switch for stuck background work: drop every session of the profile
/// and clear its cached schema (memory + disk).
#[tauri::command]
async fn background_kill(
    state: State<'_, Arc<AppState>>,
    cache: State<'_, Arc<CatalogCache>>,
    profile_id: String,
) -> AppResult<()> {
    let state = state.inner().clone();
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.close_profile_sessions(&profile_id);
        cache.mem.lock().remove(&profile_id);
        cache.refreshed.lock().remove(&profile_id);
        CatalogCache::disk_clear(&profile_id);
        Ok(())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

// ---------------------------------------------------------------- profiles --

#[tauri::command]
fn profiles_list(state: State<'_, Arc<AppState>>) -> Vec<ConnectionProfile> {
    state.profiles.lock().clone()
}

#[tauri::command]
fn profile_save(state: State<'_, Arc<AppState>>, mut profile: ConnectionProfile) -> AppResult<()> {
    // Real passwords go to the OS keychain; the file keeps a sentinel. If the
    // keychain refuses (locked/headless), fall back to storing plaintext so
    // the credential is never lost.
    let id = profile.id.clone();
    if let Some(pw) = profile.password_mut() {
        if pw == secrets::SENTINEL {
            // unchanged — keychain already holds the real value
        } else if pw.is_empty() {
            secrets::delete(&id);
        } else if secrets::store(&id, pw) {
            *pw = secrets::SENTINEL.to_string();
        }
    }
    {
        let mut profiles = state.profiles.lock();
        if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile;
        } else {
            profiles.push(profile);
        }
    }
    state.save_profiles()
}

/// Persist a new ordering of the connection list (drag-reorder in the tree).
/// Profiles missing from `ids` keep their relative order at the end, so a
/// stale UI can never drop a connection.
#[tauri::command]
fn profiles_reorder(state: State<'_, Arc<AppState>>, ids: Vec<String>) -> AppResult<()> {
    {
        let mut profiles = state.profiles.lock();
        profiles.sort_by_key(|p| {
            ids.iter().position(|id| *id == p.id).unwrap_or(usize::MAX)
        });
    }
    state.save_profiles()
}

#[tauri::command]
async fn profile_delete(state: State<'_, Arc<AppState>>, profile_id: String) -> AppResult<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.close_profile_sessions(&profile_id);
        state.profiles.lock().retain(|p| p.id != profile_id);
        // Deleting a connection also removes its local console files,
        // history entries and keychain credential.
        let _ = consoles::delete_all(&profile_id);
        let _ = history::purge_profile(&profile_id);
        secrets::delete(&profile_id);
        state.save_profiles()
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn profile_disconnect(
    state: State<'_, Arc<AppState>>,
    profile_id: String,
) -> AppResult<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.close_profile_sessions(&profile_id);
        Ok(())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
fn sessions_count(state: State<'_, Arc<AppState>>) -> HashMap<String, usize> {
    state.sessions_count()
}

#[tauri::command]
async fn databases_list(
    state: State<'_, Arc<AppState>>,
    profile_id: String,
) -> AppResult<Vec<String>> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_meta_retry(&state, &profile_id, None, |conn| conn.list_databases())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn connection_test(profile: ConnectionProfile) -> AppResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = drivers::connect(&profile)?;
        conn.ping()?;
        Ok("Connection OK".to_string())
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

// ------------------------------------------------------------------- query --

#[derive(Serialize)]
struct StatementOutcome {
    sql: String,
    kind: String, // "rows" | "affected" | "ok"
    #[serde(rename = "resultId")]
    result_id: Option<u64>,
    columns: Option<Vec<ColumnMeta>>,
    #[serde(rename = "rowCount")]
    row_count: usize,
    truncated: bool,
    affected: Option<u64>,
    #[serde(rename = "elapsedMs")]
    elapsed_ms: u64,
}

#[derive(Serialize)]
struct RunError {
    message: String,
    kind: error::ErrorKind,
    #[serde(rename = "statementIndex")]
    statement_index: usize,
}

#[derive(Serialize)]
struct RunResponse {
    statements: Vec<StatementOutcome>,
    error: Option<RunError>,
    #[serde(rename = "totalElapsedMs")]
    total_elapsed_ms: u64,
}

/// True when an execution error smells like a dead/broken connection (server
/// restarted, idle timeout, network drop) rather than a SQL problem.
fn is_connection_error(message: &str) -> bool {
    let m = message.to_lowercase();
    [
        "connection closed",
        "connection reset",
        "connection refused",
        "broken pipe",
        "server has gone away",
        "communication link failure",
        "connection is closed",
        "terminating connection",
        "unexpected eof",
        "connection unexpectedly",
        "no connection to the server",
        "i/o error",
        "socket",
        "08s01",
        "57p01",
        "epipe",
    ]
    .iter()
    .any(|p| m.contains(p))
}

#[tauri::command]
async fn query_run(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    activity: State<'_, Arc<ActivityRegistry>>,
    profile_id: String,
    tab_id: String,
    sql: String,
    max_rows: usize,
    database: Option<String>,
) -> AppResult<RunResponse> {
    let state = state.inner().clone();
    let activity = activity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        let label: String = sql.split_whitespace().collect::<Vec<_>>().join(" ");
        let label: String = label.chars().take(80).collect();
        let act = activity.begin(&app, "query", &profile_id, &label);
        // Ensure the activity entry clears on every exit path.
        struct ActGuard<'a>(&'a Arc<ActivityRegistry>, &'a tauri::AppHandle, u64);
        impl Drop for ActGuard<'_> {
            fn drop(&mut self) {
                self.0.end(self.1, self.2);
            }
        }
        let _guard = ActGuard(&activity, &app, act);
        let key = state::session_key(&profile_id, &tab_id);
        let flag = state.cancel_flag(&key);
        flag.store(false, Ordering::Relaxed);

        let statements = sqlsplit::split_statements(&sql);
        if statements.is_empty() {
            return Ok(RunResponse {
                statements: vec![],
                error: None,
                total_elapsed_ms: 0,
            });
        }

        let profile_name = state
            .profile(&profile_id)
            .map(|p| p.name)
            .unwrap_or_else(|_| profile_id.clone());
        let mut outcomes: Vec<StatementOutcome> = Vec::new();
        let mut run_error: Option<RunError> = None;
        let mut history_entries: Vec<history::HistoryEntry> = Vec::new();

        let mut reconnect_used = false;
        for (idx, stmt) in statements.iter().enumerate() {
            let stmt_start = std::time::Instant::now();
            let mut columns: Vec<ColumnMeta> = Vec::new();
            let mut rows: Vec<Vec<DbValue>> = Vec::new();

            let db = database.as_deref();
            let run_once = |columns: &mut Vec<ColumnMeta>, rows: &mut Vec<Vec<DbValue>>| {
                state.with_conn(&profile_id, &tab_id, db, |conn| {
                    let mut on_event = |ev: RowEvent| -> ControlFlow<()> {
                        match ev {
                            RowEvent::Columns(c) => {
                                *columns = c;
                                ControlFlow::Continue(())
                            }
                            RowEvent::Row(r) => {
                                rows.push(r);
                                if max_rows > 0 && rows.len() >= max_rows {
                                    ControlFlow::Break(())
                                } else {
                                    ControlFlow::Continue(())
                                }
                            }
                        }
                    };
                    conn.execute(&stmt.sql, &flag, &mut on_event)
                })
            };

            let mut exec = run_once(&mut columns, &mut rows);
            // Mixed state: we hold a session but the server dropped it.
            // Reset the session and retry the statement once.
            if let Err(e) = &exec {
                if e.kind != error::ErrorKind::Cancelled
                    && !reconnect_used
                    && is_connection_error(&e.message)
                {
                    reconnect_used = true;
                    state.reset_session(&profile_id, &tab_id);
                    columns.clear();
                    rows.clear();
                    exec = run_once(&mut columns, &mut rows);
                }
            }

            let elapsed_ms = stmt_start.elapsed().as_millis() as u64;
            let preview: String = stmt.sql.chars().take(120).collect();

            match exec {
                Ok(meta) => {
                    if meta.columns.is_some() || !columns.is_empty() {
                        let cols = meta.columns.unwrap_or_else(|| columns.clone());
                        let row_count = rows.len();
                        let truncated = meta.stopped_early && max_rows > 0 && row_count >= max_rows;
                        let result_id = state.store_result(cols.clone(), std::mem::take(&mut rows));
                        history_entries.push(history::HistoryEntry {
                            ts: history::now_iso(),
                            profile_id: profile_id.clone(),
                            profile_name: profile_name.clone(),
                            sql: stmt.sql.clone(),
                            kind: "rows".into(),
                            row_count: Some(row_count),
                            affected: None,
                            elapsed_ms,
                            error: None,
                        });
                        outcomes.push(StatementOutcome {
                            sql: preview,
                            kind: "rows".into(),
                            result_id: Some(result_id),
                            columns: Some(cols),
                            row_count,
                            truncated,
                            affected: None,
                            elapsed_ms,
                        });
                    } else {
                        let kind = if meta.affected.is_some() { "affected" } else { "ok" };
                        history_entries.push(history::HistoryEntry {
                            ts: history::now_iso(),
                            profile_id: profile_id.clone(),
                            profile_name: profile_name.clone(),
                            sql: stmt.sql.clone(),
                            kind: kind.into(),
                            row_count: None,
                            affected: meta.affected,
                            elapsed_ms,
                            error: None,
                        });
                        outcomes.push(StatementOutcome {
                            sql: preview,
                            kind: kind.into(),
                            result_id: None,
                            columns: None,
                            row_count: 0,
                            truncated: false,
                            affected: meta.affected,
                            elapsed_ms,
                        });
                    }
                }
                Err(e) => {
                    history_entries.push(history::HistoryEntry {
                        ts: history::now_iso(),
                        profile_id: profile_id.clone(),
                        profile_name: profile_name.clone(),
                        sql: stmt.sql.clone(),
                        kind: "error".into(),
                        row_count: None,
                        affected: None,
                        elapsed_ms,
                        error: Some(e.message.clone()),
                    });
                    run_error = Some(RunError {
                        message: e.message,
                        kind: e.kind,
                        statement_index: idx,
                    });
                    break;
                }
            }
        }

        if let Err(e) = history::append(&history_entries) {
            eprintln!("history append failed: {}", e.message);
        }

        Ok(RunResponse {
            statements: outcomes,
            error: run_error,
            total_elapsed_ms: started.elapsed().as_millis() as u64,
        })
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn query_cancel(
    state: State<'_, Arc<AppState>>,
    profile_id: String,
    tab_id: String,
) -> AppResult<()> {
    let state = state.inner().clone();
    // Server-side cancel (e.g. PostgreSQL) opens a network connection; keep it
    // off the main thread.
    tauri::async_runtime::spawn_blocking(move || state.cancel_session(&profile_id, &tab_id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))
}

#[derive(Serialize)]
struct Page {
    rows: Vec<Vec<DbValue>>,
}

#[tauri::command]
fn page_fetch(
    state: State<'_, Arc<AppState>>,
    result_id: u64,
    offset: usize,
    limit: usize,
) -> AppResult<Page> {
    state.with_result(result_id, |r| {
        let end = (offset + limit).min(r.rows.len());
        let start = offset.min(end);
        Page { rows: r.rows[start..end].to_vec() }
    })
}

#[tauri::command]
fn results_free(state: State<'_, Arc<AppState>>, result_ids: Vec<u64>) {
    state.free_results(&result_ids);
}

#[tauri::command]
async fn session_close(
    state: State<'_, Arc<AppState>>,
    profile_id: String,
    tab_id: String,
) -> AppResult<()> {
    let state = state.inner().clone();
    // Closing waits for any statement still running on the session.
    tauri::async_runtime::spawn_blocking(move || state.close_session(&profile_id, &tab_id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))
}

// ----------------------------------------------------------------- catalog --


/// Run a metadata operation on the shared "meta" session, transparently
/// reconnecting once if the cached connection died (server restart, idle
/// timeout). Queries get this via query_run's own retry; catalog/object
/// introspection needs it too or the tree keeps showing a stale error.
fn with_meta_retry<T>(
    state: &Arc<AppState>,
    profile_id: &str,
    database: Option<&str>,
    f: impl Fn(&mut Box<dyn drivers::DbConnection>) -> AppResult<T>,
) -> AppResult<T> {
    match state.with_conn(profile_id, "meta", database, &f) {
        Err(e) if is_connection_error(&e.to_string()) => {
            state.reset_session(profile_id, "meta");
            state.with_conn(profile_id, "meta", database, &f)
        }
        r => r,
    }
}

/// Refresh a profile's catalog from the live server in the background,
/// updating the caches and notifying the frontend when done. At most one
/// per profile per app run (the `refreshed` guard).
fn spawn_catalog_refresh(
    app: tauri::AppHandle,
    state: Arc<AppState>,
    cache: Arc<CatalogCache>,
    activity: Arc<ActivityRegistry>,
    profile_id: String,
) {
    {
        let mut refreshed = cache.refreshed.lock();
        if refreshed.contains(&profile_id) {
            return;
        }
        refreshed.insert(profile_id.clone());
    }
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Emitter;
        let name = state
            .profile(&profile_id)
            .map(|p| p.name)
            .unwrap_or_else(|_| profile_id.clone());
        let act = activity.begin(&app, "background", &profile_id, &format!("Refreshing schema — {name}"));
        let result = with_meta_retry(&state, &profile_id, None, |conn| conn.catalog());
        activity.end(&app, act);
        if let Ok(cat) = result {
            cache.put(&profile_id, &cat);
            let _ = app.emit("catalog:updated", profile_id);
        }
    });
}

#[tauri::command]
async fn catalog_get(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    cache: State<'_, Arc<CatalogCache>>,
    activity: State<'_, Arc<ActivityRegistry>>,
    profile_id: String,
    refresh: bool,
) -> AppResult<Catalog> {
    let state = state.inner().clone();
    let cache = cache.inner().clone();
    let activity = activity.inner().clone();
    if !refresh {
        if let Some(c) = cache.mem.lock().get(&profile_id) {
            return Ok(c.clone());
        }
        // Disk cache: instant tree, then refresh from the live server in the
        // background so it heals staleness without blocking the UI.
        let pid = profile_id.clone();
        let from_disk =
            tauri::async_runtime::spawn_blocking(move || CatalogCache::disk_load(&pid))
                .await
                .map_err(|e| AppError::internal(e.to_string()))?;
        if let Some(cat) = from_disk {
            cache.mem.lock().insert(profile_id.clone(), cat.clone());
            spawn_catalog_refresh(app, state, cache, activity, profile_id);
            return Ok(cat);
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        let name = state
            .profile(&profile_id)
            .map(|p| p.name)
            .unwrap_or_else(|_| profile_id.clone());
        let act = activity.begin(&app, "background", &profile_id, &format!("Loading schema — {name}"));
        let result = with_meta_retry(&state, &profile_id, None, |conn| conn.catalog());
        activity.end(&app, act);
        let cat = result?;
        cache.put(&profile_id, &cat);
        cache.refreshed.lock().insert(profile_id);
        Ok(cat)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[derive(Serialize)]
struct DdlResponse {
    ddl: String,
    native: bool,
}

#[tauri::command]
async fn ddl_get(
    state: State<'_, Arc<AppState>>,
    cache: State<'_, Arc<CatalogCache>>,
    profile_id: String,
    schema: String,
    table: String,
) -> AppResult<DdlResponse> {
    let state = state.inner().clone();
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let native =
            with_meta_retry(&state, &profile_id, None, |conn| conn.native_ddl(&schema, &table))?;
        if let Some(ddl) = native {
            return Ok(DdlResponse {
                ddl: format!("{}{}", ddl::ddl_comment(true), ddl),
                native: true,
            });
        }
        // Generate from catalog metadata (cached, or refreshed on miss).
        let dialect = state.with_conn(&profile_id, "meta", None, |conn| Ok(conn.dialect()))?;
        let mut cat = cache.mem.lock().get(&profile_id).cloned();
        if cat
            .as_ref()
            .map(|c| find_table(c, &schema, &table).is_none())
            .unwrap_or(true)
        {
            let fresh = with_meta_retry(&state, &profile_id, None, |conn| conn.catalog())?;
            cache.put(&profile_id, &fresh);
            cat = Some(fresh);
        }
        let cat = cat.unwrap_or_default();
        let info = find_table(&cat, &schema, &table)
            .ok_or_else(|| AppError::query(format!("table {schema}.{table} not found")))?;
        Ok(DdlResponse {
            ddl: format!(
                "{}{}",
                ddl::ddl_comment(false),
                ddl::generate_table_ddl(dialect, &schema, info)
            ),
            native: false,
        })
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

fn find_table<'a>(cat: &'a Catalog, schema: &str, table: &str) -> Option<&'a catalog::TableInfo> {
    cat.schemas
        .iter()
        .find(|s| s.name == schema)?
        .tables
        .iter()
        .find(|t| t.name == table)
}

// ---------------------------------------------------------------- transfer --

#[tauri::command]
async fn import_preview(
    path: String,
    delimiter: Option<String>,
    has_header: Option<bool>,
) -> AppResult<transfer::ImportPreview> {
    tauri::async_runtime::spawn_blocking(move || {
        transfer::import_preview(&path, delimiter, has_header)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn import_run(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    activity: State<'_, Arc<ActivityRegistry>>,
    req: transfer::ImportRequest,
) -> AppResult<transfer::ImportResult> {
    let state = state.inner().clone();
    let activity = activity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let act = activity.begin(
            &app,
            "background",
            &req.profile_id,
            &format!("Importing into {}", req.table),
        );
        let flag = {
            let mut jobs = state.job_flags.lock();
            jobs.entry(req.job_id.clone())
                .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
                .clone()
        };
        flag.store(false, Ordering::Relaxed);
        let tab = format!("job:{}", req.job_id);
        let result =
            state.with_conn(&req.profile_id, &tab, None, |conn| {
                transfer::import_run(&app, conn, &req, &flag)
            });
        state.close_session(&req.profile_id, &tab);
        state.job_flags.lock().remove(&req.job_id);
        activity.end(&app, act);
        result
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn export_run(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    activity: State<'_, Arc<ActivityRegistry>>,
    req: transfer::ExportRequest,
) -> AppResult<transfer::ExportResult> {
    let state = state.inner().clone();
    let activity = activity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let act = activity.begin(
            &app,
            "background",
            &req.profile_id,
            &format!("Exporting {}", req.table_name),
        );
        let flag = {
            let mut jobs = state.job_flags.lock();
            jobs.entry(req.job_id.clone())
                .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
                .clone()
        };
        flag.store(false, Ordering::Relaxed);
        let tab = format!("job:{}", req.job_id);
        let result =
            state.with_conn(&req.profile_id, &tab, req.database.as_deref(), |conn| {
                transfer::export_run(&app, conn, &req, &flag)
            });
        state.close_session(&req.profile_id, &tab);
        state.job_flags.lock().remove(&req.job_id);
        activity.end(&app, act);
        result
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
fn job_cancel(state: State<'_, Arc<AppState>>, job_id: String) {
    if let Some(flag) = state.job_flags.lock().get(&job_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

// ----------------------------------------------------------------- history --

#[tauri::command]
async fn history_list(
    limit: Option<usize>,
    filter: Option<String>,
    profile_id: Option<String>,
) -> AppResult<Vec<history::HistoryEntry>> {
    tauri::async_runtime::spawn_blocking(move || {
        history::list(limit.unwrap_or(500), filter, profile_id)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn history_clear() -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(history::clear)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}


// ----------------------------------------------------------- schema objects --

#[tauri::command]
async fn objects_list(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    activity: State<'_, Arc<ActivityRegistry>>,
    profile_id: String,
    database: Option<String>,
) -> AppResult<Vec<objects::DbObject>> {
    let state = state.inner().clone();
    let activity = activity.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let name = state
            .profile(&profile_id)
            .map(|p| p.name)
            .unwrap_or_else(|_| profile_id.clone());
        let act = activity.begin(&app, "background", &profile_id, &format!("Loading objects — {name}"));
        let r = with_meta_retry(&state, &profile_id, database.as_deref(), |conn| {
            objects::list(conn.as_mut())
        });
        activity.end(&app, act);
        r
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn object_ddl(
    state: State<'_, Arc<AppState>>,
    profile_id: String,
    database: Option<String>,
    kind: String,
    schema: Option<String>,
    name: String,
    detail: Option<String>,
) -> AppResult<String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_meta_retry(&state, &profile_id, database.as_deref(), |conn| {
            objects::ddl(
                conn.as_mut(),
                &kind,
                schema.as_deref().unwrap_or(""),
                &name,
                detail.as_deref().unwrap_or(""),
            )
        })
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

// ---------------------------------------------------------------- consoles --

#[tauri::command]
async fn console_save(console: consoles::ConsoleFile) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || consoles::save(&console))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn console_list(profile_id: Option<String>) -> AppResult<Vec<consoles::ConsoleFile>> {
    tauri::async_runtime::spawn_blocking(move || consoles::list(profile_id.as_deref()))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn console_delete(profile_id: String, console_id: String) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || consoles::delete(&profile_id, &console_id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

// ----------------------------------------------------------- session state --

fn session_state_path() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("QueryDeck")
        .join("session.json")
}

/// Persist the open-console layout (opaque JSON owned by the frontend) so a
/// relaunch restores the same set of consoles.
#[tauri::command]
async fn session_state_save(json: String) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = session_state_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(AppError::from)?;
        }
        std::fs::write(&path, json).map_err(AppError::from)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn session_state_load() -> AppResult<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || Ok(std::fs::read_to_string(session_state_path()).ok()))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

// -------------------------------------------------------------- app config --

fn app_config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("QueryDeck")
        .join("config.json")
}

/// Persistent application configuration (formatter options, etc.). The JSON
/// is owned by the frontend; the backend just stores it durably.
#[tauri::command]
async fn config_save(json: String) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = app_config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(AppError::from)?;
        }
        std::fs::write(&path, json).map_err(AppError::from)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn config_load() -> AppResult<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || Ok(std::fs::read_to_string(app_config_path()).ok()))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

// ----------------------------------------------------------------- tunnels --

#[tauri::command]
fn tunnels_list() -> Vec<tunnels::TunnelConfig> {
    tunnels::list()
}

#[tauri::command]
fn tunnel_save(tunnel: tunnels::TunnelConfig) -> AppResult<()> {
    tunnels::save(tunnel)
}

#[tauri::command]
fn tunnel_delete(id: String) -> AppResult<()> {
    tunnels::delete(&id)
}

#[tauri::command]
async fn tunnel_start(id: String) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || tunnels::ensure(&id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
fn tunnel_stop(id: String) {
    tunnels::stop(&id);
}

#[tauri::command]
fn tunnels_status() -> Vec<tunnels::TunnelStatus> {
    tunnels::status()
}

// ---------------------------------------------------------------- registry --

#[tauri::command]
fn drivers_registry(_state: State<'_, Arc<AppState>>) -> Vec<registry::DriverPackageStatus> {
    registry::list()
}

#[tauri::command]
async fn driver_download(app: tauri::AppHandle, id: String) -> AppResult<String> {
    tauri::async_runtime::spawn_blocking(move || registry::download(&app, &id))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

#[derive(Serialize)]
struct EnvStatus {
    #[serde(rename = "odbcAvailable")]
    odbc_available: bool,
    #[serde(rename = "odbcDetail")]
    odbc_detail: String,
    #[serde(rename = "jvmAvailable")]
    jvm_available: bool,
    #[serde(rename = "jvmDetail")]
    jvm_detail: String,
}

#[tauri::command]
fn env_status() -> EnvStatus {
    let (odbc_available, odbc_detail) = drivers::odbc::status();
    let (jvm_available, jvm_detail) = drivers::jdbc::status();
    EnvStatus { odbc_available, odbc_detail, jvm_available, jvm_detail }
}

#[tauri::command]
async fn odbc_inventory() -> AppResult<drivers::odbc::OdbcInventory> {
    tauri::async_runtime::spawn_blocking(drivers::odbc::inventory)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
fn drivers_dir() -> String {
    let dir = AppState::drivers_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir.to_string_lossy().into_owned()
}

// --------------------------------------------------------------------- run --

pub fn run() {
    let boot = std::time::Instant::now();
    let state = Arc::new(AppState::default());
    state.load_profiles();
    eprintln!("[startup] profiles loaded in {:?}", boot.elapsed());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .manage(Arc::new(CatalogCache::default()))
        .manage(Arc::new(ActivityRegistry::default()))
        .setup(move |app| {
            use tauri::menu::{Menu, MenuItemBuilder, MenuItemKind, PredefinedMenuItem, SubmenuBuilder};
            let show_history = MenuItemBuilder::with_id("show-history", "Show Query History…")
                .accelerator("CmdOrCtrl+Y")
                .build(app)?;
            let history_menu = SubmenuBuilder::new(app, "History").item(&show_history).build()?;

            let menu = Menu::default(app.handle())?;
            // Default order is App, File, Edit, View, Window, Help.
            // Requested order: File, Edit, View, History, Window, Help —
            // insert History right before Window.
            let items = menu.items()?;
            let window_pos = items
                .iter()
                .position(|i| matches!(i, MenuItemKind::Submenu(s) if s.text().ok().as_deref() == Some("Window")))
                .unwrap_or(items.len());
            menu.insert(&history_menu, window_pos)?;

            // ⌘W belongs to "close the current console tab" (and only closes
            // the window once no consoles remain) — drop the predefined
            // Close Window item so its accelerator doesn't shadow ours.
            if let Some(MenuItemKind::Submenu(window_menu)) = items
                .iter()
                .find(|i| matches!(i, MenuItemKind::Submenu(s) if s.text().ok().as_deref() == Some("Window")))
            {
                let wi = window_menu.items()?;
                if let Some(pos) = wi.iter().position(
                    |i| matches!(i, MenuItemKind::Predefined(p) if p.text().ok().as_deref() == Some("Close Window")),
                ) {
                    let _ = window_menu.remove_at(pos);
                }
            }

            // Edit gains the query reformatter (and its settings).
            if let Some(MenuItemKind::Submenu(edit_menu)) = items
                .iter()
                .find(|i| matches!(i, MenuItemKind::Submenu(s) if s.text().ok().as_deref() == Some("Edit")))
            {
                let reformat = MenuItemBuilder::with_id("reformat-query", "Reformat Query")
                    .accelerator("CmdOrCtrl+Alt+L")
                    .build(app)?;
                let fmt_settings =
                    MenuItemBuilder::with_id("formatter-settings", "Formatter Settings…")
                        .build(app)?;
                edit_menu.append(&PredefinedMenuItem::separator(app)?)?;
                edit_menu.append(&reformat)?;
                edit_menu.append(&fmt_settings)?;
            }

            // Tunnels & drivers live under File (sidebar buttons were removed).
            if let Some(MenuItemKind::Submenu(file_menu)) = items
                .iter()
                .find(|i| matches!(i, MenuItemKind::Submenu(s) if s.text().ok().as_deref() == Some("File")))
            {
                let close_console = MenuItemBuilder::with_id("close-console", "Close Console")
                    .accelerator("CmdOrCtrl+W")
                    .build(app)?;
                let tunnels_item =
                    MenuItemBuilder::with_id("show-tunnels", "SSH Tunnels…").build(app)?;
                let drivers_item =
                    MenuItemBuilder::with_id("show-drivers", "Database Drivers…").build(app)?;
                file_menu.prepend(&close_console)?;
                file_menu.append(&PredefinedMenuItem::separator(app)?)?;
                file_menu.append(&tunnels_item)?;
                file_menu.append(&drivers_item)?;
            }

            app.set_menu(menu)?;
            eprintln!("[startup] setup complete in {:?}", boot.elapsed());
            app.on_menu_event(|app, event| {
                use tauri::Emitter;
                match event.id().as_ref() {
                    "show-history" => {
                        let _ = app.emit("open-history", ());
                    }
                    "show-tunnels" => {
                        let _ = app.emit("open-tunnels", ());
                    }
                    "show-drivers" => {
                        let _ = app.emit("open-drivers", ());
                    }
                    "close-console" => {
                        let _ = app.emit("close-console", ());
                    }
                    "reformat-query" => {
                        let _ = app.emit("reformat-query", ());
                    }
                    "formatter-settings" => {
                        let _ = app.emit("open-formatter-settings", ());
                    }
                    _ => {}
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            profiles_list,
            profile_save,
            profiles_reorder,
            profile_delete,
            connection_test,
            query_run,
            query_cancel,
            page_fetch,
            results_free,
            session_close,
            catalog_get,
            ddl_get,
            objects_list,
            object_ddl,
            import_preview,
            import_run,
            export_run,
            job_cancel,
            drivers_registry,
            driver_download,
            env_status,
            history_list,
            history_clear,
            profile_disconnect,
            sessions_count,
            databases_list,
            console_save,
            console_list,
            console_delete,
            session_state_save,
            session_state_load,
            config_save,
            config_load,
            tunnels_list,
            tunnel_save,
            tunnel_delete,
            tunnel_start,
            tunnel_stop,
            tunnels_status,
            odbc_inventory,
            drivers_dir,
            activity_list,
            background_kill
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                let t = std::time::Instant::now();
                tunnels::stop_all();
                eprintln!("[shutdown] tunnels stopped in {:?}", t.elapsed());
            }
        });
}
