//! Persistent query history: one JSON line per executed statement, appended
//! to `<data dir>/QueryDeck/history.jsonl`. Lightweight by design — we keep
//! what was *run* (statement, connection, outcome), not an edit log.

use std::io::Write;
use std::path::PathBuf;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Compact once the file grows past this size…
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// …keeping this many most-recent entries.
const KEEP_ON_COMPACT: usize = 5_000;

static FILE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// ISO-8601 local timestamp.
    pub ts: String,
    #[serde(rename = "profileId")]
    pub profile_id: String,
    #[serde(rename = "profileName")]
    pub profile_name: String,
    pub sql: String,
    /// "rows" | "affected" | "ok" | "error"
    pub kind: String,
    #[serde(rename = "rowCount", skip_serializing_if = "Option::is_none")]
    pub row_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected: Option<u64>,
    #[serde(rename = "elapsedMs")]
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn history_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("QueryDeck")
        .join("history.jsonl")
}

pub fn append(entries: &[HistoryEntry]) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let _guard = FILE_LOCK.lock();
    let path = history_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut buf = String::new();
    for e in entries {
        buf.push_str(&serde_json::to_string(e).map_err(|e| AppError::internal(e.to_string()))?);
        buf.push('\n');
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    f.write_all(buf.as_bytes())?;
    drop(f);

    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES {
        compact(&path)?;
    }
    Ok(())
}

fn compact(path: &PathBuf) -> AppResult<()> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    let keep = lines.len().saturating_sub(KEEP_ON_COMPACT);
    let tmp = path.with_extension("jsonl.tmp");
    let mut out = String::new();
    for line in &lines[keep..] {
        out.push_str(line);
        out.push('\n');
    }
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Most recent entries first. `filter` is a case-insensitive substring match
/// on the SQL and profile name; `profile_id` narrows to one connection.
pub fn list(
    limit: usize,
    filter: Option<String>,
    profile_id: Option<String>,
) -> AppResult<Vec<HistoryEntry>> {
    let _guard = FILE_LOCK.lock();
    let path = history_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let needle = filter.map(|f| f.to_lowercase()).filter(|f| !f.is_empty());
    let mut out: Vec<HistoryEntry> = Vec::with_capacity(limit.min(1024));
    for line in content.lines().rev() {
        if out.len() >= limit {
            break;
        }
        let Ok(entry) = serde_json::from_str::<HistoryEntry>(line) else {
            continue;
        };
        if let Some(pid) = &profile_id {
            if &entry.profile_id != pid {
                continue;
            }
        }
        if let Some(n) = &needle {
            if !entry.sql.to_lowercase().contains(n)
                && !entry.profile_name.to_lowercase().contains(n)
            {
                continue;
            }
        }
        out.push(entry);
    }
    Ok(out)
}

/// Drop every history entry belonging to one profile (connection deleted).
pub fn purge_profile(profile_id: &str) -> AppResult<()> {
    let _guard = FILE_LOCK.lock();
    let path = history_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let mut out = String::new();
    for line in content.lines() {
        match serde_json::from_str::<HistoryEntry>(line) {
            Ok(e) if e.profile_id == profile_id => continue,
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    std::fs::write(&path, out)?;
    Ok(())
}

pub fn clear() -> AppResult<()> {
    let _guard = FILE_LOCK.lock();
    let path = history_path();
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
