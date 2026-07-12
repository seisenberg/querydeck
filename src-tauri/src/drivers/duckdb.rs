//! DuckDB, natively via the official embedded engine (duckdb-rs, bundled).
//! Far faster than going through the JDBC bridge, and needs no JVM.

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use duckdb::types::ValueRef;
use duckdb::Connection;

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, DbConnection, Dialect, DuckdbParams, ExecMeta, RowEvent, RowSink};

pub struct DuckdbConn {
    conn: Connection,
}

impl DuckdbConn {
    pub fn open(params: &DuckdbParams) -> AppResult<Self> {
        let conn = if params.path.trim().is_empty() {
            Connection::open_in_memory()
        } else {
            Connection::open(&params.path)
        }
        .map_err(|e| AppError::connect(format!("DuckDB: {e}")))?;
        Ok(Self { conn })
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

fn cell(v: ValueRef<'_>) -> DbValue {
    use duckdb::types::TimeUnit;
    match v {
        ValueRef::Null => DbValue::Null,
        ValueRef::Boolean(b) => DbValue::Bool(b),
        ValueRef::TinyInt(i) => DbValue::Int(i as i64),
        ValueRef::SmallInt(i) => DbValue::Int(i as i64),
        ValueRef::Int(i) => DbValue::Int(i as i64),
        ValueRef::BigInt(i) => DbValue::Int(i),
        ValueRef::HugeInt(i) => DbValue::Text(i.to_string()),
        ValueRef::UTinyInt(i) => DbValue::Int(i as i64),
        ValueRef::USmallInt(i) => DbValue::Int(i as i64),
        ValueRef::UInt(i) => DbValue::Int(i as i64),
        ValueRef::UBigInt(i) => {
            if i <= i64::MAX as u64 {
                DbValue::Int(i as i64)
            } else {
                DbValue::Text(i.to_string())
            }
        }
        ValueRef::Float(f) => DbValue::float(f as f64),
        ValueRef::Double(f) => DbValue::float(f),
        ValueRef::Decimal(d) => DbValue::Text(d.to_string()),
        ValueRef::Text(t) => DbValue::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => DbValue::bytes(b),
        ValueRef::Date32(days) => {
            match chrono::DateTime::from_timestamp(days as i64 * 86_400, 0) {
                Some(dt) => DbValue::Text(dt.date_naive().to_string()),
                None => DbValue::Null,
            }
        }
        ValueRef::Time64(unit, v) => {
            let micros = match unit {
                TimeUnit::Second => v * 1_000_000,
                TimeUnit::Millisecond => v * 1_000,
                TimeUnit::Microsecond => v,
                TimeUnit::Nanosecond => v / 1_000,
            };
            let secs = (micros / 1_000_000) as u32;
            let t = chrono::NaiveTime::from_num_seconds_from_midnight_opt(
                secs,
                ((micros % 1_000_000) * 1_000) as u32,
            );
            match t {
                Some(t) => DbValue::Text(t.to_string()),
                None => DbValue::Null,
            }
        }
        ValueRef::Timestamp(unit, v) => {
            let (secs, nanos) = match unit {
                TimeUnit::Second => (v, 0u32),
                TimeUnit::Millisecond => (v.div_euclid(1_000), (v.rem_euclid(1_000) * 1_000_000) as u32),
                TimeUnit::Microsecond => (v.div_euclid(1_000_000), (v.rem_euclid(1_000_000) * 1_000) as u32),
                TimeUnit::Nanosecond => (v.div_euclid(1_000_000_000), v.rem_euclid(1_000_000_000) as u32),
            };
            match chrono::DateTime::from_timestamp(secs, nanos) {
                Some(dt) => DbValue::Text(dt.naive_utc().format("%Y-%m-%d %H:%M:%S%.f").to_string()),
                None => DbValue::Null,
            }
        }
        // Nested/exotic types (LIST, STRUCT, MAP, ENUM, INTERVAL, …) render
        // through their debug form — visible, if not pretty.
        other => DbValue::Text(format!("{other:?}")),
    }
}

/// Statements that produce result sets get streamed; everything else runs
/// through execute() so changed-row counts come back. DuckDB allows
/// FROM-first queries, so that leads too.
fn returns_rows(sql: &str) -> bool {
    let head = sql
        .trim_start()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    matches!(
        head.as_str(),
        "select" | "with" | "values" | "from" | "show" | "describe" | "explain" | "summarize"
            | "pragma" | "call" | "table"
    ) || sql.to_lowercase().contains(" returning ")
}

impl DbConnection for DuckdbConn {
    fn dialect(&self) -> Dialect {
        Dialect::Generic
    }

    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_row: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        if !returns_rows(sql) {
            let affected = self
                .conn
                .execute(sql, [])
                .map_err(|e| AppError::query(format!("DuckDB: {e}")))?;
            return Ok(ExecMeta {
                columns: None,
                affected: Some(affected as u64),
                stopped_early: false,
            });
        }

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| AppError::query(format!("DuckDB: {e}")))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| AppError::query(format!("DuckDB: {e}")))?;

        let columns: Vec<ColumnMeta> = rows
            .as_ref()
            .map(|st| {
                st.column_names()
                    .into_iter()
                    .map(|n| ColumnMeta::new(n, ""))
                    .collect()
            })
            .unwrap_or_default();
        let ncols = columns.len();

        let mut stopped_early = false;
        if let ControlFlow::Break(()) = on_row(RowEvent::Columns(columns.clone())) {
            return Ok(ExecMeta { columns: Some(columns), affected: None, stopped_early: true });
        }
        loop {
            check_cancel(cancel)?;
            let row = match rows.next().map_err(|e| AppError::query(e.to_string()))? {
                Some(r) => r,
                None => break,
            };
            let mut out = Vec::with_capacity(ncols);
            for i in 0..ncols {
                let vr = row.get_ref(i).map_err(|e| AppError::query(e.to_string()))?;
                out.push(cell(vr));
            }
            if let ControlFlow::Break(()) = on_row(RowEvent::Row(out)) {
                stopped_early = true;
                break;
            }
        }

        Ok(ExecMeta { columns: Some(columns), affected: None, stopped_early })
    }

    fn catalog(&mut self) -> AppResult<Catalog> {
        use std::collections::BTreeMap;

        let rels = self.text_rows(
            "SELECT schema_name, table_name, 'table' FROM duckdb_tables() \
             WHERE database_name = current_database() \
             UNION ALL \
             SELECT schema_name, view_name, 'view' FROM duckdb_views() \
             WHERE database_name = current_database() AND NOT internal \
             ORDER BY 1, 2",
        )?;
        let cols = self
            .text_rows(
                "SELECT schema_name, table_name, column_name, data_type, is_nullable, column_default \
                 FROM duckdb_columns() WHERE database_name = current_database() \
                 ORDER BY schema_name, table_name, column_index",
            )
            .unwrap_or_default();
        let pks = self
            .text_rows(
                "SELECT schema_name, table_name, unnest(constraint_column_names) \
                 FROM duckdb_constraints() \
                 WHERE constraint_type = 'PRIMARY KEY' AND database_name = current_database()",
            )
            .unwrap_or_default();

        let get = |row: &[Option<String>], i: usize| -> String {
            row.get(i).and_then(|v| v.clone()).unwrap_or_default()
        };
        let mut schemas: BTreeMap<String, BTreeMap<String, TableInfo>> = BTreeMap::new();
        for row in rels {
            let (ns, name, kind) = (get(&row, 0), get(&row, 1), get(&row, 2));
            schemas
                .entry(ns)
                .or_default()
                .insert(name.clone(), TableInfo::new(name, &kind));
        }
        for row in cols {
            let (ns, tbl) = (get(&row, 0), get(&row, 1));
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                let default = get(&row, 5);
                t.columns.push(ColumnInfo {
                    name: get(&row, 2),
                    data_type: get(&row, 3),
                    nullable: get(&row, 4) == "true",
                    default: (!default.is_empty()).then_some(default),
                });
            }
        }
        for row in pks {
            let (ns, tbl) = (get(&row, 0), get(&row, 1));
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                t.pk.push(get(&row, 2));
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
        let esc = |s: &str| s.replace('\'', "''");
        let rows = self.text_rows(&format!(
            "SELECT sql FROM duckdb_tables() WHERE schema_name = '{0}' AND table_name = '{1}' \
             UNION ALL \
             SELECT sql FROM duckdb_views() WHERE schema_name = '{0}' AND view_name = '{1}'",
            esc(schema),
            esc(table)
        ))?;
        Ok(rows
            .first()
            .and_then(|r| r.first().cloned().flatten())
            .filter(|s| !s.trim().is_empty())
            .map(|s| if s.trim_end().ends_with(';') { s } else { format!("{s};") }))
    }
}
