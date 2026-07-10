use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use mysql::prelude::Queryable;
use mysql::{Conn, Opts, OptsBuilder, SslOpts, Value};

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, DbConnection, Dialect, ExecMeta, MysqlParams, RowEvent, RowSink};

pub struct MysqlConn {
    conn: Conn,
}

impl MysqlConn {
    pub fn open(params: &MysqlParams) -> AppResult<Self> {
        let mut builder = OptsBuilder::new()
            .ip_or_hostname(Some(params.host.clone()))
            .tcp_port(params.port)
            .user(Some(params.user.clone()))
            .pass(Some(params.password.clone()));
        if !params.database.is_empty() {
            builder = builder.db_name(Some(params.database.clone()));
        }
        if params.use_ssl {
            let ssl = SslOpts::default()
                .with_danger_accept_invalid_certs(params.accept_invalid_certs);
            builder = builder.ssl_opts(Some(ssl));
        }
        let opts: Opts = builder.into();
        let conn = Conn::new(opts).map_err(|e| AppError::connect(format!("MySQL: {e}")))?;
        Ok(Self { conn })
    }

    fn text_rows(&mut self, sql: &str) -> AppResult<Vec<Vec<Option<String>>>> {
        let rows: Vec<mysql::Row> = self
            .conn
            .query(sql)
            .map_err(|e| AppError::query(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (0..row.len())
                    .map(|i| match row.as_ref(i) {
                        Some(Value::NULL) | None => None,
                        Some(v) => Some(value_to_text(v)),
                    })
                    .collect()
            })
            .collect())
    }
}

fn value_to_text(v: &Value) -> String {
    match v {
        Value::NULL => String::new(),
        Value::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(d) => d.to_string(),
        Value::Date(y, m, d, h, mi, s, us) => {
            if *us > 0 {
                format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}.{us:06}")
            } else if *h > 0 || *mi > 0 || *s > 0 {
                format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
            } else {
                format!("{y:04}-{m:02}-{d:02}")
            }
        }
        Value::Time(neg, d, h, m, s, us) => {
            let sign = if *neg { "-" } else { "" };
            let hours = (*d as u64) * 24 + (*h as u64);
            if *us > 0 {
                format!("{sign}{hours:02}:{m:02}:{s:02}.{us:06}")
            } else {
                format!("{sign}{hours:02}:{m:02}:{s:02}")
            }
        }
    }
}

fn cell(v: &Value) -> DbValue {
    match v {
        Value::NULL => DbValue::Null,
        Value::Bytes(b) => match std::str::from_utf8(b) {
            Ok(s) => DbValue::Text(s.to_string()),
            Err(_) => DbValue::bytes(b),
        },
        Value::Int(i) => DbValue::Int(*i),
        Value::UInt(u) => i64::try_from(*u)
            .map(DbValue::Int)
            .unwrap_or_else(|_| DbValue::Text(u.to_string())),
        Value::Float(f) => DbValue::float(*f as f64),
        Value::Double(d) => DbValue::float(*d),
        other => DbValue::Text(value_to_text(other)),
    }
}

impl DbConnection for MysqlConn {
    fn dialect(&self) -> Dialect {
        Dialect::Mysql
    }

    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_row: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        let mut result = self
            .conn
            .query_iter(sql)
            .map_err(|e| AppError::query(e.to_string()))?;

        let mut columns: Option<Vec<ColumnMeta>> = None;
        let mut affected: Option<u64> = None;
        let mut stopped_early = false;

        // Take the first result set that has columns; report affected rows
        // otherwise. Remaining sets are drained by QueryResult's Drop.
        while let Some(rs) = result.iter() {
            let cols = rs.columns();
            let cols = cols.as_ref();
            if cols.is_empty() {
                affected = Some(rs.affected_rows());
                continue;
            }
            let metas: Vec<ColumnMeta> = cols
                .iter()
                .map(|c| {
                    let ty = format!("{:?}", c.column_type())
                        .trim_start_matches("MYSQL_TYPE_")
                        .to_ascii_lowercase();
                    ColumnMeta::new(c.name_str().into_owned(), ty)
                })
                .collect();
            columns = Some(metas.clone());
            if let ControlFlow::Break(()) = on_row(RowEvent::Columns(metas)) {
                stopped_early = true;
            }
            for row in rs {
                check_cancel(cancel)?;
                let row = row.map_err(|e| AppError::query(e.to_string()))?;
                if stopped_early {
                    continue;
                }
                let vals: Vec<DbValue> = (0..row.len())
                    .map(|i| row.as_ref(i).map(cell).unwrap_or(DbValue::Null))
                    .collect();
                if let ControlFlow::Break(()) = on_row(RowEvent::Row(vals)) {
                    stopped_early = true;
                }
            }
            break;
        }

        Ok(ExecMeta { columns, affected, stopped_early })
    }

    fn list_databases(&mut self) -> AppResult<Vec<String>> {
        // SHOW DATABASES only lists schemas the user has some privilege on.
        let rows = self.text_rows("SHOW DATABASES")?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.into_iter().next().flatten())
            .filter(|d| {
                !matches!(d.as_str(), "information_schema" | "performance_schema" | "sys")
            })
            .collect())
    }

    fn catalog(&mut self) -> AppResult<Catalog> {
        const HIDDEN: &str = "('information_schema','mysql','performance_schema','sys')";

        let tables = self.text_rows(&format!(
            "SELECT table_schema, table_name, table_type FROM information_schema.tables \
             WHERE table_schema NOT IN {HIDDEN} ORDER BY table_schema, table_name"
        ))?;
        let cols = self.text_rows(&format!(
            "SELECT table_schema, table_name, column_name, column_type, is_nullable, \
                    column_default, column_key \
             FROM information_schema.columns \
             WHERE table_schema NOT IN {HIDDEN} \
             ORDER BY table_schema, table_name, ordinal_position"
        ))?;

        let mut schemas: BTreeMap<String, BTreeMap<String, TableInfo>> = BTreeMap::new();
        for row in tables {
            let ns = text_at(&row, 0);
            let name = text_at(&row, 1);
            let kind = if text_at(&row, 2).contains("VIEW") { "view" } else { "table" };
            schemas
                .entry(ns)
                .or_default()
                .insert(name.clone(), TableInfo::new(name, kind));
        }
        for row in cols {
            let ns = text_at(&row, 0);
            let tbl = text_at(&row, 1);
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                let cname = text_at(&row, 2);
                if text_at(&row, 6) == "PRI" {
                    t.pk.push(cname.clone());
                }
                t.columns.push(ColumnInfo {
                    name: cname,
                    data_type: text_at(&row, 3),
                    nullable: text_at(&row, 4) == "YES",
                    default: row.get(5).and_then(|v| v.clone()),
                });
            }
        }

        Ok(Catalog {
            schemas: schemas
                .into_iter()
                .map(|(name, tables)| SchemaInfo {
                    name,
                    tables: tables.into_values().collect(),
                })
                .collect(),
        })
    }

    fn native_ddl(&mut self, schema: &str, table: &str) -> AppResult<Option<String>> {
        let q = Dialect::Mysql;
        let sql = format!(
            "SHOW CREATE TABLE {}.{}",
            q.quote_ident(schema),
            q.quote_ident(table)
        );
        let rows = self.text_rows(&sql)?;
        if let Some(row) = rows.first() {
            // SHOW CREATE TABLE: (name, ddl); SHOW CREATE VIEW: (name, ddl, charset, collation)
            if let Some(Some(ddl)) = row.get(1) {
                return Ok(Some(format!("{ddl};")));
            }
        }
        Ok(None)
    }
}

fn text_at(row: &[Option<String>], i: usize) -> String {
    row.get(i).and_then(|v| v.clone()).unwrap_or_default()
}
