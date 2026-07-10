use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use rusqlite::types::ValueRef;
use rusqlite::Connection;

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, DbConnection, Dialect, ExecMeta, RowEvent, RowSink, SqliteParams};

pub struct SqliteConn {
    conn: Connection,
}

impl SqliteConn {
    pub fn open(params: &SqliteParams) -> AppResult<Self> {
        let conn = Connection::open(&params.path)
            .map_err(|e| AppError::connect(format!("SQLite: {e}")))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| AppError::connect(e.to_string()))?;
        Ok(Self { conn })
    }
}

fn cell(v: ValueRef<'_>) -> DbValue {
    match v {
        ValueRef::Null => DbValue::Null,
        ValueRef::Integer(i) => DbValue::Int(i),
        ValueRef::Real(f) => DbValue::float(f),
        ValueRef::Text(t) => DbValue::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => DbValue::bytes(b),
    }
}

impl DbConnection for SqliteConn {
    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_row: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| AppError::query(e.to_string()))?;
        let ncols = stmt.column_count();

        if ncols == 0 {
            let affected = stmt.raw_execute().map_err(|e| AppError::query(e.to_string()))?;
            return Ok(ExecMeta {
                columns: None,
                affected: Some(affected as u64),
                stopped_early: false,
            });
        }

        let columns: Vec<ColumnMeta> = stmt
            .columns()
            .iter()
            .map(|c| ColumnMeta::new(c.name(), c.decl_type().unwrap_or("")))
            .collect();

        let mut stopped_early = false;
        if let ControlFlow::Break(()) = on_row(RowEvent::Columns(columns.clone())) {
            return Ok(ExecMeta { columns: Some(columns), affected: None, stopped_early: true });
        }
        let mut rows = stmt.raw_query();
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
        let mut schema = SchemaInfo { name: "main".into(), tables: Vec::new() };

        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, type FROM sqlite_master \
                 WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .map_err(|e| AppError::query(e.to_string()))?;
        let names: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| AppError::query(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);

        for (name, kind) in names {
            let mut table = TableInfo::new(&name, if kind == "view" { "view" } else { "table" });
            let quoted = name.replace('\'', "''");
            let mut cols = self
                .conn
                .prepare(&format!("PRAGMA table_info('{quoted}')"))
                .map_err(|e| AppError::query(e.to_string()))?;
            let infos: Vec<(String, String, i64, Option<String>, i64)> = cols
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                })
                .map_err(|e| AppError::query(e.to_string()))?
                .filter_map(|r| r.ok())
                .collect();
            for (cname, ctype, notnull, dflt, pk) in infos {
                if pk > 0 {
                    table.pk.push(cname.clone());
                }
                table.columns.push(ColumnInfo {
                    name: cname,
                    data_type: ctype,
                    nullable: notnull == 0,
                    default: dflt,
                });
            }
            schema.tables.push(table);
        }

        Ok(Catalog { schemas: vec![schema] })
    }

    fn native_ddl(&mut self, _schema: &str, table: &str) -> AppResult<Option<String>> {
        let mut parts: Vec<String> = Vec::new();
        let mut stmt = self
            .conn
            .prepare("SELECT sql FROM sqlite_master WHERE tbl_name = ?1 AND sql IS NOT NULL ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'view' THEN 0 ELSE 1 END")
            .map_err(|e| AppError::query(e.to_string()))?;
        let sqls: Vec<String> = stmt
            .query_map([table], |r| r.get::<_, String>(0))
            .map_err(|e| AppError::query(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();
        for s in sqls {
            parts.push(format!("{s};"));
        }
        if parts.is_empty() {
            Ok(None)
        } else {
            Ok(Some(parts.join("\n\n")))
        }
    }
}
