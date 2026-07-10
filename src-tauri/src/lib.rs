mod catalog;
mod consoles;
mod ddl;
mod drivers;
mod error;
mod history;
mod registry;
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
        connect, ConnectionProfile, DbConnection, Dialect, DriverParams, RowEvent, SqliteParams,
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

/// Catalog cache, keyed by profile id.
#[derive(Default)]
struct CatalogCache(Mutex<HashMap<String, Catalog>>);

// ---------------------------------------------------------------- profiles --

#[tauri::command]
fn profiles_list(state: State<'_, Arc<AppState>>) -> Vec<ConnectionProfile> {
    state.profiles.lock().clone()
}

#[tauri::command]
fn profile_save(state: State<'_, Arc<AppState>>, profile: ConnectionProfile) -> AppResult<()> {
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

#[tauri::command]
async fn profile_delete(state: State<'_, Arc<AppState>>, profile_id: String) -> AppResult<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.close_profile_sessions(&profile_id);
        state.profiles.lock().retain(|p| p.id != profile_id);
        // Deleting a connection also removes its local console files and
        // history entries.
        let _ = consoles::delete_all(&profile_id);
        let _ = history::purge_profile(&profile_id);
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
        state.with_conn(&profile_id, "meta", None, |conn| conn.list_databases())
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
    state: State<'_, Arc<AppState>>,
    profile_id: String,
    tab_id: String,
    sql: String,
    max_rows: usize,
    database: Option<String>,
) -> AppResult<RunResponse> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
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

#[tauri::command]
async fn catalog_get(
    state: State<'_, Arc<AppState>>,
    cache: State<'_, Arc<CatalogCache>>,
    profile_id: String,
    refresh: bool,
) -> AppResult<Catalog> {
    if !refresh {
        if let Some(c) = cache.0.lock().get(&profile_id) {
            return Ok(c.clone());
        }
    }
    let state = state.inner().clone();
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cat = state.with_conn(&profile_id, "meta", None, |conn| conn.catalog())?;
        cache.0.lock().insert(profile_id, cat.clone());
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
            state.with_conn(&profile_id, "meta", None, |conn| conn.native_ddl(&schema, &table))?;
        if let Some(ddl) = native {
            return Ok(DdlResponse {
                ddl: format!("{}{}", ddl::ddl_comment(true), ddl),
                native: true,
            });
        }
        // Generate from catalog metadata (cached, or refreshed on miss).
        let dialect = state.with_conn(&profile_id, "meta", None, |conn| Ok(conn.dialect()))?;
        let mut cat = cache.0.lock().get(&profile_id).cloned();
        if cat
            .as_ref()
            .map(|c| find_table(c, &schema, &table).is_none())
            .unwrap_or(true)
        {
            let fresh = state.with_conn(&profile_id, "meta", None, |conn| conn.catalog())?;
            cache.0.lock().insert(profile_id.clone(), fresh.clone());
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
    req: transfer::ImportRequest,
) -> AppResult<transfer::ImportResult> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
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
        result
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

#[tauri::command]
async fn export_run(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    req: transfer::ExportRequest,
) -> AppResult<transfer::ExportResult> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
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
    let state = Arc::new(AppState::default());
    state.load_profiles();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .manage(Arc::new(CatalogCache::default()))
        .setup(|app| {
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

            // Tunnels & drivers live under File (sidebar buttons were removed).
            if let Some(MenuItemKind::Submenu(file_menu)) = items
                .iter()
                .find(|i| matches!(i, MenuItemKind::Submenu(s) if s.text().ok().as_deref() == Some("File")))
            {
                let tunnels_item =
                    MenuItemBuilder::with_id("show-tunnels", "SSH Tunnels…").build(app)?;
                let drivers_item =
                    MenuItemBuilder::with_id("show-drivers", "Database Drivers…").build(app)?;
                file_menu.append(&PredefinedMenuItem::separator(app)?)?;
                file_menu.append(&tunnels_item)?;
                file_menu.append(&drivers_item)?;
            }

            app.set_menu(menu)?;
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
                    _ => {}
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            profiles_list,
            profile_save,
            profile_delete,
            connection_test,
            query_run,
            query_cancel,
            page_fetch,
            results_free,
            session_close,
            catalog_get,
            ddl_get,
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
            tunnels_list,
            tunnel_save,
            tunnel_delete,
            tunnel_start,
            tunnel_stop,
            tunnels_status,
            odbc_inventory,
            drivers_dir
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                tunnels::stop_all();
            }
        });
}
