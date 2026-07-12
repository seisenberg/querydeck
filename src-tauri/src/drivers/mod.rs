pub mod sqlite;
pub mod postgres;
pub mod mysql;
pub mod mssql;
pub mod clickhouse;
pub mod odbc;
pub mod jdbc;

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

/// Saved connection profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    /// Optional SSH tunnel id (see tunnels.rs) that must be up before
    /// connecting. The connection itself targets the tunnel's local port.
    #[serde(rename = "sshTunnel", default, skip_serializing_if = "Option::is_none")]
    pub ssh_tunnel: Option<String>,
    #[serde(flatten)]
    pub params: DriverParams,
}

impl ConnectionProfile {
    /// A copy of this profile pointing at a different database (used by the
    /// per-tab database switcher). Drivers without a database concept are
    /// returned unchanged.
    pub fn with_database(&self, database: Option<&str>) -> ConnectionProfile {
        let mut p = self.clone();
        if let Some(db) = database {
            if !db.is_empty() {
                match &mut p.params {
                    DriverParams::Postgres(pg) | DriverParams::Redshift(pg) => {
                        pg.database = db.to_string()
                    }
                    DriverParams::Mysql(my) | DriverParams::Mariadb(my) => {
                        my.database = db.to_string()
                    }
                    DriverParams::Mssql(ms) => ms.database = db.to_string(),
                    DriverParams::Clickhouse(ch) => ch.database = db.to_string(),
                    _ => {}
                }
            }
        }
        p
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "driver", content = "params", rename_all = "snake_case")]
pub enum DriverParams {
    Sqlite(SqliteParams),
    Postgres(PostgresParams),
    /// Amazon Redshift speaks the Postgres wire protocol; it shares
    /// PostgresParams but gets tolerant catalog queries (PG 8 era views).
    Redshift(PostgresParams),
    Mysql(MysqlParams),
    /// MariaDB shares the MySQL wire protocol and parameters.
    Mariadb(MysqlParams),
    Mssql(MssqlParams),
    Clickhouse(ClickhouseParams),
    Odbc(OdbcParams),
    Jdbc(JdbcParams),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteParams {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub password: String,
    pub database: String,
    /// "disable" | "prefer" | "require"
    #[serde(default = "default_ssl_mode")]
    pub ssl_mode: String,
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

fn default_ssl_mode() -> String {
    "prefer".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MysqlParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub use_ssl: bool,
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MssqlParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub database: String,
    /// Require TLS on the connection (SQL Server encrypts by default).
    #[serde(default = "default_true")]
    pub encrypt: bool,
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickhouseParams {
    pub host: String,
    /// HTTP interface port (8123 plain, 8443 TLS).
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub use_https: bool,
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OdbcParams {
    /// Full ODBC connection string, e.g. `Driver={...};Server=...;` or `DSN=name;`
    pub connection_string: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JdbcParams {
    pub url: String,
    pub driver_class: String,
    /// Paths to driver jars (absolute). Registry downloads land in the app
    /// data dir and are referenced here.
    pub jars: Vec<String>,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
}

/// SQL dialect quirks needed by generic machinery (DDL generation, imports).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Dialect {
    Sqlite,
    Postgres,
    Mysql,
    Mssql,
    Clickhouse,
    Generic,
}

impl Dialect {
    pub fn quote_ident(&self, ident: &str) -> String {
        match self {
            Dialect::Mysql | Dialect::Clickhouse => format!("`{}`", ident.replace('`', "``")),
            _ => format!("\"{}\"", ident.replace('"', "\"\"")),
        }
    }

    pub fn escape_str(&self, s: &str) -> String {
        match self {
            // MySQL and ClickHouse treat backslash as an escape character.
            Dialect::Mysql | Dialect::Clickhouse => {
                s.replace('\\', "\\\\").replace('\'', "''")
            }
            _ => s.replace('\'', "''"),
        }
    }

    pub fn literal(&self, v: &DbValue) -> String {
        match v {
            DbValue::Null => "NULL".into(),
            DbValue::Bool(b) => {
                if matches!(
                    self,
                    Dialect::Sqlite | Dialect::Mysql | Dialect::Mssql | Dialect::Clickhouse
                ) {
                    if *b { "1".into() } else { "0".into() }
                } else if *b {
                    "TRUE".into()
                } else {
                    "FALSE".into()
                }
            }
            DbValue::Int(i) => i.to_string(),
            DbValue::Float(f) => f.to_string(),
            DbValue::Text(s) => format!("'{}'", self.escape_str(s)),
        }
    }
}

/// Result of executing a single statement.
#[derive(Debug)]
pub struct ExecMeta {
    /// Present when the statement produced a result set.
    pub columns: Option<Vec<ColumnMeta>>,
    /// Rows affected for DML, when known.
    pub affected: Option<u64>,
    /// True when fetching stopped early (row limit or cancel).
    pub stopped_early: bool,
}

/// Streaming events from statement execution. `Columns` is emitted once,
/// before the first `Row`, whenever the statement has a result set.
pub enum RowEvent {
    Columns(Vec<ColumnMeta>),
    Row(Vec<DbValue>),
}

pub type RowSink<'a> = dyn FnMut(RowEvent) -> ControlFlow<()> + 'a;

/// A live database connection. One per (profile, tab) so tabs work in parallel.
pub trait DbConnection: Send {
    fn dialect(&self) -> Dialect;

    /// Execute one statement. Column metadata and rows are pushed into
    /// `on_event`; a `ControlFlow::Break` return tells the driver to stop
    /// fetching. The driver should also poll `cancel` between rows.
    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_event: &mut RowSink,
    ) -> AppResult<ExecMeta>;

    /// Introspect the database structure.
    fn catalog(&mut self) -> AppResult<Catalog>;

    /// Databases the current user may connect to, for the per-tab database
    /// switcher. Empty = switching not supported for this engine.
    fn list_databases(&mut self) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }

    /// Driver-native DDL for a table when the engine can produce it
    /// (e.g. `SHOW CREATE TABLE`, sqlite_master.sql). `None` -> generate
    /// generically from catalog metadata.
    fn native_ddl(&mut self, _schema: &str, _table: &str) -> AppResult<Option<String>> {
        Ok(None)
    }

    /// A callback that cancels the currently running statement server-side,
    /// callable from another thread while this connection is busy.
    fn server_cancel(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        None
    }

    /// A cheap liveness check.
    fn ping(&mut self) -> AppResult<()> {
        let mut sink = |_ev: RowEvent| ControlFlow::Continue(());
        self.execute("SELECT 1", &AtomicBool::new(false), &mut sink)?;
        Ok(())
    }
}

/// Open a connection for a profile. Brings the profile's SSH tunnel up first
/// when one is configured.
pub fn connect(profile: &ConnectionProfile) -> AppResult<Box<dyn DbConnection>> {
    if let Some(tunnel_id) = &profile.ssh_tunnel {
        if !tunnel_id.is_empty() {
            crate::tunnels::ensure(tunnel_id)?;
        }
    }
    match &profile.params {
        DriverParams::Sqlite(p) => Ok(Box::new(sqlite::SqliteConn::open(p)?)),
        DriverParams::Postgres(p) => Ok(Box::new(postgres::PostgresConn::open(p)?)),
        DriverParams::Redshift(p) => {
            Ok(Box::new(postgres::PostgresConn::open_redshift(p)?))
        }
        DriverParams::Mysql(p) | DriverParams::Mariadb(p) => {
            Ok(Box::new(mysql::MysqlConn::open(p)?))
        }
        DriverParams::Mssql(p) => Ok(Box::new(mssql::MssqlConn::open(p)?)),
        DriverParams::Clickhouse(p) => Ok(Box::new(clickhouse::ClickhouseConn::open(p)?)),
        DriverParams::Odbc(p) => Ok(Box::new(odbc::OdbcConn::open(p)?)),
        DriverParams::Jdbc(p) => Ok(Box::new(jdbc::JdbcConn::open(p)?)),
    }
}

/// Tree grouping key for ODBC/JDBC metadata that reports catalog + schema.
pub fn odbc_schema_key(catalog: &str, schema: &str) -> String {
    match (catalog.is_empty(), schema.is_empty()) {
        (false, false) => format!("{catalog}.{schema}"),
        (false, true) => catalog.to_string(),
        (true, false) => schema.to_string(),
        (true, true) => "(default)".to_string(),
    }
}

pub fn check_cancel(cancel: &AtomicBool) -> AppResult<()> {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        Err(AppError::cancelled())
    } else {
        Ok(())
    }
}
