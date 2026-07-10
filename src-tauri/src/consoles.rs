//! Persisted query consoles. Closing a tab keeps its SQL + log on disk
//! (client-side), listed under the connection in the schema tree; clicking
//! one reopens it. Files live in `<data dir>/QueryDeck/consoles/<profile>/`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Stored console tab. `log` entries are kept as opaque JSON so the frontend
/// owns their shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleFile {
    pub id: String,
    pub title: String,
    #[serde(rename = "profileId")]
    pub profile_id: String,
    #[serde(default)]
    pub database: Option<String>,
    pub sql: String,
    #[serde(default)]
    pub log: Vec<serde_json::Value>,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    /// True when the tab is not currently open in the UI.
    #[serde(default)]
    pub closed: bool,
}

fn consoles_dir(profile_id: &str) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("QueryDeck")
        .join("consoles")
        .join(sanitize(profile_id))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

pub fn save(console: &ConsoleFile) -> AppResult<()> {
    let dir = consoles_dir(&console.profile_id);
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(console).map_err(|e| AppError::internal(e.to_string()))?;
    std::fs::write(dir.join(format!("{}.json", sanitize(&console.id))), json)?;
    Ok(())
}

/// All saved consoles for one profile (or every profile when `None`),
/// newest first.
pub fn list(profile_id: Option<&str>) -> AppResult<Vec<ConsoleFile>> {
    let root = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("QueryDeck")
        .join("consoles");
    let mut dirs_to_read: Vec<PathBuf> = Vec::new();
    match profile_id {
        Some(p) => dirs_to_read.push(root.join(sanitize(p))),
        None => {
            if let Ok(rd) = std::fs::read_dir(&root) {
                for e in rd.flatten() {
                    if e.path().is_dir() {
                        dirs_to_read.push(e.path());
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for dir in dirs_to_read {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            if e.path().extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Ok(bytes) = std::fs::read(e.path()) {
                if let Ok(c) = serde_json::from_slice::<ConsoleFile>(&bytes) {
                    out.push(c);
                }
            }
        }
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(out)
}

pub fn delete(profile_id: &str, console_id: &str) -> AppResult<()> {
    let path = consoles_dir(profile_id).join(format!("{}.json", sanitize(console_id)));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Remove every saved console for a profile (used when the connection itself
/// is deleted).
pub fn delete_all(profile_id: &str) -> AppResult<()> {
    let dir = consoles_dir(profile_id);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
