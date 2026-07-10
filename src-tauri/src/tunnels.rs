//! SSH tunnels, managed as `ssh -N -L` child processes of the system OpenSSH
//! client (present on macOS/Linux and modern Windows). A tunnel defines a
//! fixed local forward `127.0.0.1:local_port -> remote_host:remote_port`
//! through an SSH host; connections reference a tunnel by id and point their
//! host/port (or URL / connection string) at the local end.
//!
//! Auth: key file or ssh-agent/ssh-config (BatchMode). Interactive password
//! prompts are not supported.

use std::collections::HashMap;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    pub id: String,
    pub name: String,
    #[serde(rename = "sshHost")]
    pub ssh_host: String,
    #[serde(rename = "sshPort", default = "default_ssh_port")]
    pub ssh_port: u16,
    #[serde(rename = "sshUser")]
    pub ssh_user: String,
    /// Optional private key path; otherwise ssh-agent / ~/.ssh/config apply.
    #[serde(rename = "keyPath", default)]
    pub key_path: Option<String>,
    #[serde(rename = "localPort")]
    pub local_port: u16,
    #[serde(rename = "remoteHost")]
    pub remote_host: String,
    #[serde(rename = "remotePort")]
    pub remote_port: u16,
}

fn default_ssh_port() -> u16 {
    22
}

#[derive(Debug, Clone, Serialize)]
pub struct TunnelStatus {
    pub id: String,
    pub running: bool,
}

static RUNNING: Lazy<Mutex<HashMap<String, Child>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("QueryDeck")
        .join("tunnels.json")
}

pub fn list() -> Vec<TunnelConfig> {
    std::fs::read(config_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_all(tunnels: &[TunnelConfig]) -> AppResult<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json =
        serde_json::to_vec_pretty(tunnels).map_err(|e| AppError::internal(e.to_string()))?;
    std::fs::write(&path, json)?;
    Ok(())
}

pub fn save(tunnel: TunnelConfig) -> AppResult<()> {
    let mut all = list();
    if let Some(existing) = all.iter_mut().find(|t| t.id == tunnel.id) {
        *existing = tunnel;
    } else {
        all.push(tunnel);
    }
    save_all(&all)
}

pub fn delete(id: &str) -> AppResult<()> {
    stop(id);
    let mut all = list();
    all.retain(|t| t.id != id);
    save_all(&all)
}

fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(400),
    )
    .is_ok()
}

fn is_alive(child: &mut Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

/// Start (or confirm) a tunnel; returns once the local port accepts
/// connections or fails with the ssh error output.
pub fn ensure(id: &str) -> AppResult<()> {
    let cfg = list()
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| AppError::config(format!("unknown SSH tunnel {id}")))?;

    {
        let mut running = RUNNING.lock();
        if let Some(child) = running.get_mut(id) {
            if is_alive(child) && port_open(cfg.local_port) {
                return Ok(());
            }
            let _ = child.kill();
            running.remove(id);
        }
    }

    let mut cmd = Command::new("ssh");
    cmd.arg("-N")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-p")
        .arg(cfg.ssh_port.to_string())
        .arg("-L")
        .arg(format!(
            "127.0.0.1:{}:{}:{}",
            cfg.local_port, cfg.remote_host, cfg.remote_port
        ));
    if let Some(key) = &cfg.key_path {
        if !key.is_empty() {
            cmd.arg("-i").arg(key);
        }
    }
    cmd.arg(format!("{}@{}", cfg.ssh_user, cfg.ssh_host));
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::driver(format!("could not run ssh: {e}")))?;

    // Wait for the forward to come up (or ssh to die with an error).
    for _ in 0..25 {
        if port_open(cfg.local_port) {
            RUNNING.lock().insert(id.to_string(), child);
            return Ok(());
        }
        if !is_alive(&mut child) {
            let mut err = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                use std::io::Read;
                let _ = stderr.read_to_string(&mut err);
            }
            let err = err.trim();
            return Err(AppError::connect(format!(
                "SSH tunnel '{}' failed: {}",
                cfg.name,
                if err.is_empty() { "ssh exited" } else { err }
            )));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    Err(AppError::connect(format!(
        "SSH tunnel '{}' did not open 127.0.0.1:{} in time",
        cfg.name, cfg.local_port
    )))
}

pub fn stop(id: &str) {
    if let Some(mut child) = RUNNING.lock().remove(id) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub fn stop_all() {
    let ids: Vec<String> = RUNNING.lock().keys().cloned().collect();
    for id in ids {
        stop(&id);
    }
}

pub fn status() -> Vec<TunnelStatus> {
    let mut running = RUNNING.lock();
    list()
        .iter()
        .map(|t| {
            let alive = running
                .get_mut(&t.id)
                .map(|c| is_alive(c) && port_open(t.local_port))
                .unwrap_or(false);
            TunnelStatus { id: t.id.clone(), running: alive }
        })
        .collect()
}
