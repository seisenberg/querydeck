use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::drivers::{self, ConnectionProfile, DbConnection};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

/// Cap on rows kept in memory across all stored result sets. Oldest results
/// are evicted first once the cap is exceeded.
const MAX_STORED_ROWS: usize = 2_000_000;

pub struct StoredResult {
    /// Kept alongside rows for future use (e.g. exporting a stored result).
    #[allow(dead_code)]
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<DbValue>>,
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
    pub fn with_conn<T>(
        &self,
        profile_id: &str,
        tab_id: &str,
        f: impl FnOnce(&mut Box<dyn DbConnection>) -> AppResult<T>,
    ) -> AppResult<T> {
        let key = session_key(profile_id, tab_id);
        let slot = {
            let mut sessions = self.sessions.lock();
            sessions.entry(key.clone()).or_default().clone()
        };
        let mut guard = slot.lock();
        if guard.is_none() {
            let profile = self.profile(profile_id)?;
            let conn = drivers::connect(&profile)?;
            if let Some(cancel) = conn.server_cancel() {
                self.server_cancels.lock().insert(key.clone(), cancel.into());
            }
            *guard = Some(conn);
        }
        f(guard.as_mut().expect("connection just initialized"))
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
        let id = self.next_result_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut store = self.results.lock();
        store.total_rows += rows.len();
        store.map.insert(id, StoredResult { columns, rows });
        store.insertion_order.push(id);
        while store.total_rows > MAX_STORED_ROWS && store.insertion_order.len() > 1 {
            let oldest = store.insertion_order.remove(0);
            if let Some(r) = store.map.remove(&oldest) {
                store.total_rows -= r.rows.len();
            }
        }
        id
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
            }
        }
        store.insertion_order.retain(|i| !ids.contains(i));
    }
}
