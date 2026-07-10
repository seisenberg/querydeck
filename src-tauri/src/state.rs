use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::drivers::{self, ConnectionProfile, DbConnection};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

/// Caps on the in-memory result store. Oldest results are evicted first once
/// either cap is exceeded — rows for pathological narrow results, bytes for
/// wide/texty ones.
const MAX_STORED_ROWS: usize = 2_000_000;
const MAX_STORED_BYTES: usize = 256 * 1024 * 1024;

pub struct StoredResult {
    /// Kept alongside rows for future use (e.g. exporting a stored result).
    #[allow(dead_code)]
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<DbValue>>,
    /// Approximate memory footprint of `rows`.
    pub approx_bytes: usize,
}

type SharedConn = Arc<Mutex<Option<Box<dyn DbConnection>>>>;

#[derive(Default)]
pub struct AppState {
    pub profiles: Mutex<Vec<ConnectionProfile>>,
    sessions: Mutex<HashMap<String, SharedConn>>,
    /// Server-side cancel callbacks (currently PostgreSQL only), usable while
    /// the session connection itself is locked by a running query.
    server_cancels: Mutex<HashMap<String, Arc<dyn Fn() + Send + Sync>>>,
    /// Cooperative cancel flags, one per session.
    pub cancel_flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Cancel flags for import/export jobs, keyed by job id.
    pub job_flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    results: Mutex<ResultStore>,
    next_result_id: AtomicU64,
}

#[derive(Default)]
struct ResultStore {
    map: HashMap<u64, StoredResult>,
    insertion_order: Vec<u64>,
    total_rows: usize,
    total_bytes: usize,
}

pub fn session_key(profile_id: &str, tab_id: &str) -> String {
    format!("{profile_id}/{tab_id}")
}

impl AppState {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("QueryDeck")
            .join("connections.json")
    }

    pub fn drivers_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("QueryDeck")
            .join("drivers")
    }

    pub fn load_profiles(&self) {
        let path = Self::config_path();
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(list) = serde_json::from_slice::<Vec<ConnectionProfile>>(&bytes) {
                *self.profiles.lock() = list;
            }
        }
    }

    pub fn save_profiles(&self) -> AppResult<()> {
        let path = Self::config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(&*self.profiles.lock())
            .map_err(|e| AppError::internal(e.to_string()))?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    pub fn profile(&self, profile_id: &str) -> AppResult<ConnectionProfile> {
        self.profiles
            .lock()
            .iter()
            .find(|p| p.id == profile_id)
            .cloned()
            .ok_or_else(|| AppError::config(format!("unknown connection profile {profile_id}")))
    }

    /// Run `f` with the session's connection, opening it on first use.
    /// `database` overrides the profile's database at connection time
    /// (per-tab database switcher).
    pub fn with_conn<T>(
        &self,
        profile_id: &str,
        tab_id: &str,
        database: Option<&str>,
        f: impl FnOnce(&mut Box<dyn DbConnection>) -> AppResult<T>,
    ) -> AppResult<T> {
        let key = session_key(profile_id, tab_id);
        let slot = {
            let mut sessions = self.sessions.lock();
            sessions.entry(key.clone()).or_default().clone()
        };
        let mut guard = slot.lock();
        if guard.is_none() {
            let profile = self.profile(profile_id)?.with_database(database);
            let conn = drivers::connect(&profile)?;
            if let Some(cancel) = conn.server_cancel() {
                self.server_cancels.lock().insert(key.clone(), cancel.into());
            }
            *guard = Some(conn);
        }
        f(guard.as_mut().expect("connection just initialized"))
    }

    /// Drop a session's connection so the next use reconnects fresh (used
    /// after detecting a broken connection).
    pub fn reset_session(&self, profile_id: &str, tab_id: &str) {
        let key = session_key(profile_id, tab_id);
        let slot = self.sessions.lock().get(&key).cloned();
        if let Some(slot) = slot {
            *slot.lock() = None;
        }
        self.server_cancels.lock().remove(&key);
    }

    /// Open session counts per profile id (for connection-state icons).
    pub fn sessions_count(&self) -> HashMap<String, usize> {
        let sessions = self.sessions.lock();
        let mut out: HashMap<String, usize> = HashMap::new();
        for (key, slot) in sessions.iter() {
            let Some((profile_id, _)) = key.split_once('/') else { continue };
            // If the inner lock is held, a query is running -> open session.
            let open = match slot.try_lock() {
                Some(guard) => guard.is_some(),
                None => true,
            };
            if open {
                *out.entry(profile_id.to_string()).or_default() += 1;
            }
        }
        out
    }

    pub fn close_session(&self, profile_id: &str, tab_id: &str) {
        let key = session_key(profile_id, tab_id);
        let slot = self.sessions.lock().remove(&key);
        self.server_cancels.lock().remove(&key);
        self.cancel_flags.lock().remove(&key);
        // Drop the connection outside the map lock; waits for a running query.
        if let Some(slot) = slot {
            *slot.lock() = None;
        }
    }

    pub fn close_profile_sessions(&self, profile_id: &str) {
        let prefix = format!("{profile_id}/");
        let keys: Vec<String> = self
            .sessions
            .lock()
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for key in keys {
            if let Some((p, t)) = key.split_once('/') {
                let (p, t) = (p.to_string(), t.to_string());
                self.close_session(&p, &t);
            }
        }
    }

    pub fn cancel_flag(&self, key: &str) -> Arc<AtomicBool> {
        self.cancel_flags
            .lock()
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    }

    /// Request cancellation of whatever is running on this session.
    pub fn cancel_session(&self, profile_id: &str, tab_id: &str) {
        let key = session_key(profile_id, tab_id);
        if let Some(flag) = self.cancel_flags.lock().get(&key) {
            flag.store(true, Ordering::Relaxed);
        }
        let cancel = self.server_cancels.lock().get(&key).cloned();
        if let Some(cancel) = cancel {
            cancel();
        }
    }

    pub fn store_result(&self, columns: Vec<ColumnMeta>, rows: Vec<Vec<DbValue>>) -> u64 {
        let approx_bytes: usize = rows
            .iter()
            .map(|r| {
                std::mem::size_of::<Vec<DbValue>>()
                    + r.iter().map(|v| v.approx_bytes()).sum::<usize>()
            })
            .sum();
        let id = self.next_result_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut store = self.results.lock();
        store.total_rows += rows.len();
        store.total_bytes += approx_bytes;
        store.map.insert(id, StoredResult { columns, rows, approx_bytes });
        store.insertion_order.push(id);
        while (store.total_rows > MAX_STORED_ROWS || store.total_bytes > MAX_STORED_BYTES)
            && store.insertion_order.len() > 1
        {
            let oldest = store.insertion_order.remove(0);
            if let Some(r) = store.map.remove(&oldest) {
                store.total_rows -= r.rows.len();
                store.total_bytes -= r.approx_bytes;
            }
        }
        id
    }

    /// Store totals: (results, rows, approx bytes) — for stress checks.
    pub fn store_stats(&self) -> (usize, usize, usize) {
        let store = self.results.lock();
        (store.map.len(), store.total_rows, store.total_bytes)
    }

    pub fn with_result<T>(
        &self,
        id: u64,
        f: impl FnOnce(&StoredResult) -> T,
    ) -> AppResult<T> {
        let store = self.results.lock();
        store
            .map
            .get(&id)
            .map(f)
            .ok_or_else(|| AppError::internal("result set no longer available (evicted)"))
    }

    pub fn free_results(&self, ids: &[u64]) {
        let mut store = self.results.lock();
        for id in ids {
            if let Some(r) = store.map.remove(id) {
                store.total_rows -= r.rows.len();
                store.total_bytes -= r.approx_bytes;
            }
        }
        store.insertion_order.retain(|i| !ids.contains(i));
    }
}
