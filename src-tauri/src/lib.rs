mod catalog;
mod ddl;
mod drivers;
mod error;
mod registry;
mod sqlsplit;
mod state;
mod transfer;
mod value;

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
fn profile_delete(state: State<'_, Arc<AppState>>, profile_id: String) -> AppResult<()> {
    state.close_profile_sessions(&profile_id);
    state.profiles.lock().retain(|p| p.id != profile_id);
    state.save_profiles()
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

#[tauri::command]
async fn query_run(
    state: State<'_, Arc<AppState>>,
    profile_id: String,
    tab_id: String,
    sql: String,
    max_rows: usize,
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

        let mut outcomes: Vec<StatementOutcome> = Vec::new();
        let mut run_error: Option<RunError> = None;

        for (idx, stmt) in statements.iter().enumerate() {
            let stmt_start = std::time::Instant::now();
            let mut columns: Vec<ColumnMeta> = Vec::new();
            let mut rows: Vec<Vec<DbValue>> = Vec::new();

            let exec = state.with_conn(&profile_id, &tab_id, |conn| {
                let mut on_event = |ev: RowEvent| -> ControlFlow<()> {
                    match ev {
                        RowEvent::Columns(c) => {
                            columns = c;
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
            });

            let elapsed_ms = stmt_start.elapsed().as_millis() as u64;
            let preview: String = stmt.sql.chars().take(120).collect();

            match exec {
                Ok(meta) => {
                    if meta.columns.is_some() || !columns.is_empty() {
                        let cols = meta.columns.unwrap_or_else(|| columns.clone());
                        let row_count = rows.len();
                        let truncated = meta.stopped_early && max_rows > 0 && row_count >= max_rows;
                        let result_id = state.store_result(cols.clone(), std::mem::take(&mut rows));
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
                        outcomes.push(StatementOutcome {
                            sql: preview,
                            kind: if meta.affected.is_some() { "affected" } else { "ok" }.into(),
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
                    run_error = Some(RunError {
                        message: e.message,
                        kind: e.kind,
                        statement_index: idx,
                    });
                    break;
                }
            }
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
fn query_cancel(state: State<'_, Arc<AppState>>, profile_id: String, tab_id: String) {
    state.cancel_session(&profile_id, &tab_id);
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
fn session_close(state: State<'_, Arc<AppState>>, profile_id: String, tab_id: String) {
    state.close_session(&profile_id, &tab_id);
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
        let cat = state.with_conn(&profile_id, "meta", |conn| conn.catalog())?;
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
            state.with_conn(&profile_id, "meta", |conn| conn.native_ddl(&schema, &table))?;
        if let Some(ddl) = native {
            return Ok(DdlResponse {
                ddl: format!("{}{}", ddl::ddl_comment(true), ddl),
                native: true,
            });
        }
        // Generate from catalog metadata (cached, or refreshed on miss).
        let dialect = state.with_conn(&profile_id, "meta", |conn| Ok(conn.dialect()))?;
        let mut cat = cache.0.lock().get(&profile_id).cloned();
        if cat
            .as_ref()
            .map(|c| find_table(c, &schema, &table).is_none())
            .unwrap_or(true)
        {
            let fresh = state.with_conn(&profile_id, "meta", |conn| conn.catalog())?;
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
            state.with_conn(&req.profile_id, &tab, |conn| {
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
            state.with_conn(&req.profile_id, &tab, |conn| {
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

// --------------------------------------------------------------------- run --

pub fn run() {
    let state = Arc::new(AppState::default());
    state.load_profiles();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .manage(Arc::new(CatalogCache::default()))
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
            env_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
