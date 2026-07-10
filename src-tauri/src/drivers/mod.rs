pub mod sqlite;
pub mod postgres;
pub mod mysql;
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
    #[serde(flatten)]
    pub params: DriverParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "driver", content = "params", rename_all = "snake_case")]
pub enum DriverParams {
    Sqlite(SqliteParams),
    Postgres(PostgresParams),
    Mysql(MysqlParams),
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
    Generic,
}

impl Dialect {
    pub fn quote_ident(&self, ident: &str) -> String {
        match self {
            Dialect::Mysql => format!("`{}`", ident.replace('`', "``")),
            _ => format!("\"{}\"", ident.replace('"', "\"\"")),
        }
    }

    pub fn escape_str(&self, s: &str) -> String {
        match self {
            // MySQL treats backslash as an escape character inside strings.
            Dialect::Mysql => s.replace('\\', "\\\\").replace('\'', "''"),
            _ => s.replace('\'', "''"),
        }
    }

    pub fn literal(&self, v: &DbValue) -> String {
        match v {
            DbValue::Null => "NULL".into(),
            DbValue::Bool(b) => {
                if *self == Dialect::Sqlite || *self == Dialect::Mysql {
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

/// Open a connection for a profile.
pub fn connect(profile: &ConnectionProfile) -> AppResult<Box<dyn DbConnection>> {
    match &profile.params {
        DriverParams::Sqlite(p) => Ok(Box::new(sqlite::SqliteConn::open(p)?)),
        DriverParams::Postgres(p) => Ok(Box::new(postgres::PostgresConn::open(p)?)),
        DriverParams::Mysql(p) => Ok(Box::new(mysql::MysqlConn::open(p)?)),
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
