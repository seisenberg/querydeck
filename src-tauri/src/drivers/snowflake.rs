//! Snowflake, natively over its HTTP REST API via snowflake-connector-rs.
//! No JVM needed. The crate is async; like mssql.rs we bridge to the
//! blocking `DbConnection` trait with a per-connection current-thread
//! tokio runtime.

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::Mutex;
use snowflake_connector_rs::{
    AuthConfig, CellValue, Client, ClientConfig, ColumnType, ExternalBrowserConfig, KeyPairConfig,
    PasswordConfig, QueryCanceller, Session, SessionConfig,
};

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, DbConnection, Dialect, ExecMeta, RowEvent, RowSink, SnowflakeParams};

pub struct SnowflakeConn {
    rt: tokio::runtime::Runtime,
    session: Session,
    params: SnowflakeParams,
    /// Canceller for the in-flight query, readable from other threads so
    /// `server_cancel` can abort it server-side (Snowflake bills warehouse
    /// time — stopping the client read alone would keep the meter running).
    canceller: Arc<Mutex<Option<QueryCanceller>>>,
}

fn sf_err(e: snowflake_connector_rs::Error) -> AppError {
    if e.is_cancelled() {
        AppError::cancelled()
    } else {
        AppError::query(format!("Snowflake: {e}"))
    }
}

fn cell(v: CellValue) -> DbValue {
    match v {
        CellValue::Null => DbValue::Null,
        CellValue::Boolean(b) => DbValue::Bool(b),
        CellValue::Integer(i) => i64::try_from(i)
            .map(DbValue::Int)
            .unwrap_or_else(|_| DbValue::Text(i.to_string())),
        CellValue::Float(f) => DbValue::float(f),
        CellValue::Decimal(d) => DbValue::Text(d.raw().to_string()),
        CellValue::String(s) => DbValue::Text(s),
        CellValue::Date(d) => DbValue::Text(d.to_string()),
        CellValue::Time(t) => DbValue::Text(t.to_string()),
        CellValue::TimestampNtz(t) => {
            DbValue::Text(t.format("%Y-%m-%d %H:%M:%S%.f").to_string())
        }
        CellValue::TimestampLtz(t) => DbValue::Text(t.to_rfc3339()),
        CellValue::TimestampTz(t) => DbValue::Text(t.to_rfc3339()),
        CellValue::Json(j) => DbValue::Text(j.to_string()),
        CellValue::Binary(b) => DbValue::bytes(b.as_bytes()),
        other => DbValue::Text(format!("{other:?}")),
    }
}

/// Human-facing column type label for the results grid.
fn type_label(ty: &ColumnType) -> String {
    match ty {
        ColumnType::Fixed { precision, scale, .. } => match (precision, scale) {
            (Some(p), Some(s)) => format!("NUMBER({p},{s})"),
            _ => "NUMBER".into(),
        },
        ColumnType::Real => "FLOAT".into(),
        ColumnType::Text { length: Some(l), .. } => format!("VARCHAR({l})"),
        ColumnType::Text { .. } => "VARCHAR".into(),
        other => other.as_str().to_uppercase(),
    }
}

/// An identifier as it appears inside a SQL statement. Simple names are left
/// unquoted so the server's case-insensitive (upper-folding) resolution
/// applies — a user typing "analytics" still reaches ANALYTICS. Anything
/// else is quoted exactly.
fn ident(name: &str) -> String {
    let simple = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if simple {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

impl SnowflakeConn {
    pub fn open(params: &SnowflakeParams) -> AppResult<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| AppError::internal(e.to_string()))?;

        let auth = match params.auth_method.as_str() {
            "password" => {
                let mut config = PasswordConfig::new(&params.password);
                if !params.passcode.is_empty() {
                    config = config.with_passcode(&params.passcode);
                }
                AuthConfig::password(config)
            }
            "keypair" => {
                let pem = std::fs::read_to_string(&params.private_key_path).map_err(|e| {
                    AppError::config(format!(
                        "Snowflake: cannot read private key {}: {e}",
                        params.private_key_path
                    ))
                })?;
                if params.passphrase.is_empty() {
                    AuthConfig::key_pair(KeyPairConfig::from_pem(pem))
                } else {
                    AuthConfig::key_pair(KeyPairConfig::from_encrypted_pem(
                        pem,
                        params.passphrase.as_bytes(),
                    ))
                }
            }
            "external_browser" => AuthConfig::external_browser(ExternalBrowserConfig::default()),
            "oauth" => AuthConfig::oauth(&params.token),
            other => {
                return Err(AppError::config(format!(
                    "Snowflake: unknown auth method \"{other}\""
                )))
            }
        };

        let mut session_config = SessionConfig::new();
        if !params.warehouse.is_empty() {
            session_config = session_config.with_warehouse(&params.warehouse);
        }
        if !params.database.is_empty() {
            session_config = session_config.with_database(&params.database);
        }
        if !params.schema.is_empty() {
            session_config = session_config.with_schema(&params.schema);
        }
        if !params.role.is_empty() {
            session_config = session_config.with_role(&params.role);
        }

        let client = Client::new(
            ClientConfig::new(&params.user, &params.account, auth).with_session(session_config),
        )
        .map_err(|e| AppError::connect(format!("Snowflake: {e}")))?;

        // External-browser auth opens the system browser here — connecting
        // is user-initiated, so the prompt lands while they're watching.
        let session = rt
            .block_on(client.create_session())
            .map_err(|e| AppError::connect(format!("Snowflake: {e}")))?;

        Ok(Self {
            rt,
            session,
            params: params.clone(),
            canceller: Arc::new(Mutex::new(None)),
        })
    }

    fn text_rows(&mut self, sql: &str) -> AppResult<Vec<Vec<Option<String>>>> {
        let cancel = AtomicBool::new(false);
        let mut rows: Vec<Vec<Option<String>>> = Vec::new();
        let mut sink = |ev: RowEvent| -> ControlFlow<()> {
            if let RowEvent::Row(r) = ev {
                rows.push(
                    r.into_iter()
                        .map(|v| match v {
                            DbValue::Null => None,
                            DbValue::Text(s) => Some(s),
                            DbValue::Bool(b) => Some(b.to_string()),
                            DbValue::Int(i) => Some(i.to_string()),
                            DbValue::Float(f) => Some(f.to_string()),
                        })
                        .collect(),
                );
            }
            ControlFlow::Continue(())
        };
        self.execute(sql, &cancel, &mut sink)?;
        Ok(rows)
    }
}

impl DbConnection for SnowflakeConn {
    fn dialect(&self) -> Dialect {
        Dialect::Snowflake
    }

    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_event: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        check_cancel(cancel)?;
        let handle = self.session.query_handle(sql.to_string()).map_err(sf_err)?;
        *self.canceller.lock() = Some(handle.canceller());

        let result = self.rt.block_on(async {
            let mut cursor = handle.execute().await.map_err(sf_err)?;

            let mut columns: Option<Vec<ColumnMeta>> = None;
            let mut stopped_early = false;
            let metas: Vec<ColumnMeta> = cursor
                .schema()
                .columns()
                .iter()
                .map(|c| ColumnMeta::new(c.name(), type_label(c.ty())))
                .collect();
            if !metas.is_empty() {
                columns = Some(metas.clone());
                if let ControlFlow::Break(()) = on_event(RowEvent::Columns(metas)) {
                    stopped_early = true;
                }
            }

            'tables: while !stopped_early {
                let Some(table) = cursor.next_table().await.map_err(sf_err)? else {
                    break;
                };
                for row in table.dynamic_rows().map_err(sf_err)? {
                    check_cancel(cancel)?;
                    let row = row.map_err(sf_err)?;
                    let (_, values) = row.into_parts();
                    let vals: Vec<DbValue> = values.into_vec().into_iter().map(cell).collect();
                    if let ControlFlow::Break(()) = on_event(RowEvent::Row(vals)) {
                        stopped_early = true;
                        continue 'tables; // re-check loop condition, stop fetching
                    }
                }
            }

            Ok(ExecMeta { columns, affected: None, stopped_early })
        });

        *self.canceller.lock() = None;
        result
    }

    fn server_cancel(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        let slot = Arc::clone(&self.canceller);
        Some(Box::new(move || {
            // Clone out of the lock so the abort request never holds it.
            let canceller = slot.lock().clone();
            if let Some(c) = canceller {
                // The connection's own runtime is blocked driving the query;
                // a throwaway runtime drives the abort request instead.
                if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    let _ = rt.block_on(c.cancel());
                }
            }
        }))
    }

    fn list_databases(&mut self) -> AppResult<Vec<String>> {
        // SHOW TERSE DATABASES: created_on, name, kind, database_name, schema_name.
        let rows = self.text_rows("SHOW TERSE DATABASES")?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.into_iter().nth(1).flatten())
            .collect())
    }

    fn catalog(&mut self) -> AppResult<Catalog> {
        use std::collections::BTreeMap;

        if self.params.database.is_empty() {
            // Without a database there is no schema to walk; the tree shows
            // the available databases instead (pick one on the connection).
            let dbs = self.list_databases()?;
            return Ok(Catalog {
                schemas: dbs
                    .into_iter()
                    .map(|d| SchemaInfo {
                        name: format!("{d} (database — set it on the connection)"),
                        tables: vec![],
                    })
                    .collect(),
            });
        }

        let db = ident(&self.params.database);
        let rels = self.text_rows(&format!(
            "SELECT table_schema, table_name, table_type FROM {db}.information_schema.tables \
             WHERE table_schema <> 'INFORMATION_SCHEMA' ORDER BY 1, 2"
        ))?;
        let cols = self
            .text_rows(&format!(
                "SELECT table_schema, table_name, column_name, data_type, is_nullable, column_default \
                 FROM {db}.information_schema.columns \
                 WHERE table_schema <> 'INFORMATION_SCHEMA' \
                 ORDER BY table_schema, table_name, ordinal_position"
            ))
            .unwrap_or_default();

        let get = |row: &[Option<String>], i: usize| -> String {
            row.get(i).and_then(|v| v.clone()).unwrap_or_default()
        };
        let mut schemas: BTreeMap<String, BTreeMap<String, TableInfo>> = BTreeMap::new();
        for row in rels {
            let (ns, name, kind) = (get(&row, 0), get(&row, 1), get(&row, 2));
            let kind = if kind.eq_ignore_ascii_case("view") { "view" } else { "table" };
            schemas
                .entry(ns)
                .or_default()
                .insert(name.clone(), TableInfo::new(name, kind));
        }
        for row in cols {
            let (ns, tbl) = (get(&row, 0), get(&row, 1));
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                let default = get(&row, 5);
                t.columns.push(ColumnInfo {
                    name: get(&row, 2),
                    data_type: get(&row, 3),
                    nullable: get(&row, 4).eq_ignore_ascii_case("yes"),
                    default: (!default.is_empty()).then_some(default),
                });
            }
        }

        Ok(Catalog {
            schemas: schemas
                .into_iter()
                .map(|(name, tables)| SchemaInfo { name, tables: tables.into_values().collect() })
                .collect(),
        })
    }

    fn native_ddl(&mut self, schema: &str, table: &str) -> AppResult<Option<String>> {
        // GET_DDL wants the object type to match; try TABLE, then VIEW.
        let name = format!(
            "\"{}\".\"{}\"",
            schema.replace('"', "\"\""),
            table.replace('"', "\"\"")
        );
        let esc = name.replace('\'', "''");
        for kind in ["TABLE", "VIEW"] {
            if let Ok(rows) = self.text_rows(&format!("SELECT GET_DDL('{kind}', '{esc}')")) {
                if let Some(ddl) = rows.first().and_then(|r| r.first().cloned().flatten()) {
                    return Ok(Some(ddl.trim().to_string()));
                }
            }
        }
        Ok(None)
    }

    fn ping(&mut self) -> AppResult<()> {
        let rows = self.text_rows("SELECT 1")?;
        if rows.is_empty() {
            return Err(AppError::connect("Snowflake: unexpected ping response"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snowflake_dialect_quoting() {
        let d = Dialect::Snowflake;
        assert_eq!(d.quote_ident("MyTable"), "\"MyTable\"");
        assert_eq!(d.quote_ident("we\"ird"), "\"we\"\"ird\"");
        // Backslash is an escape character in Snowflake string literals.
        assert_eq!(d.escape_str("a\\b'c"), "a\\\\b''c");
        assert_eq!(d.literal(&DbValue::Bool(true)), "TRUE");
        assert_eq!(d.literal(&DbValue::Bool(false)), "FALSE");
    }

    #[test]
    fn simple_idents_stay_unquoted() {
        assert_eq!(ident("ANALYTICS"), "ANALYTICS");
        assert_eq!(ident("my_db$1"), "my_db$1");
        assert_eq!(ident("Weird Name"), "\"Weird Name\"");
        assert_eq!(ident("1st"), "\"1st\"");
        assert_eq!(ident("qu\"ote"), "\"qu\"\"ote\"");
    }

    #[test]
    fn cells_map_to_db_values() {
        assert_eq!(cell(CellValue::Null), DbValue::Null);
        assert_eq!(cell(CellValue::Boolean(true)), DbValue::Bool(true));
        assert_eq!(cell(CellValue::Integer(42)), DbValue::Int(42));
        // Beyond-i64 NUMBERs degrade to text instead of truncating.
        assert_eq!(
            cell(CellValue::Integer(i128::from(i64::MAX) + 1)),
            DbValue::Text("9223372036854775808".into())
        );
        assert_eq!(cell(CellValue::Float(1.5)), DbValue::Float(1.5));
        assert_eq!(cell(CellValue::String("hi".into())), DbValue::Text("hi".into()));
        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 23).unwrap();
        assert_eq!(cell(CellValue::Date(date)), DbValue::Text("2026-07-23".into()));
        assert_eq!(
            cell(CellValue::Json(serde_json::json!({"a": 1}))),
            DbValue::Text("{\"a\":1}".into())
        );
    }

    #[test]
    fn type_labels_read_naturally() {
        // Fixed/Text are non-exhaustive variants we can't construct here;
        // the unit variants cover the as_str-based fallback path.
        assert_eq!(type_label(&ColumnType::Real), "FLOAT");
        assert_eq!(type_label(&ColumnType::Boolean), "BOOLEAN");
        assert_eq!(type_label(&ColumnType::Variant), "VARIANT");
    }
}
