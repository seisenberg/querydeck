//! ClickHouse over its HTTP interface (reqwest, blocking).
//!
//! Queries are sent with `default_format=JSONCompact`, which only kicks in
//! when the statement has no FORMAT clause of its own — so SELECTs come back
//! as JSON we can type, DDL/DML come back empty, and users who write their
//! own FORMAT still get raw text.

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, ClickhouseParams, DbConnection, Dialect, ExecMeta, RowEvent, RowSink};

pub struct ClickhouseConn {
    http: reqwest::blocking::Client,
    base: String,
    user: String,
    password: String,
    database: String,
}

#[derive(serde::Deserialize)]
struct ChMeta {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
}

#[derive(serde::Deserialize)]
struct ChResponse {
    meta: Vec<ChMeta>,
    data: Vec<Vec<serde_json::Value>>,
}

impl ClickhouseConn {
    pub fn open(params: &ClickhouseParams) -> AppResult<Self> {
        let scheme = if params.use_https { "https" } else { "http" };
        let base = format!("{scheme}://{}:{}/", params.host, params.port);
        let http = reqwest::blocking::Client::builder()
            .danger_accept_invalid_certs(params.accept_invalid_certs)
            .timeout(Duration::from_secs(600))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| AppError::connect(e.to_string()))?;
        let conn = Self {
            http,
            base,
            user: params.user.clone(),
            password: params.password.clone(),
            database: params.database.clone(),
        };
        Ok(conn)
    }

    fn request(&self, sql: &str) -> AppResult<String> {
        let mut url = format!("{}?default_format=JSONCompact", self.base);
        if !self.database.is_empty() {
            url.push_str("&database=");
            url.push_str(&pct(&self.database));
        }
        let mut req = self.http.post(&url).body(sql.to_string());
        if !self.user.is_empty() {
            req = req.header("X-ClickHouse-User", &self.user);
        }
        if !self.password.is_empty() {
            req = req.header("X-ClickHouse-Key", &self.password);
        }
        let resp = req
            .send()
            .map_err(|e| AppError::connect(format!("ClickHouse: {e}")))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if !status.is_success() {
            return Err(AppError::query(format!(
                "ClickHouse: {}",
                body.trim().is_empty().then(|| status.to_string()).unwrap_or(body.trim().to_string())
            )));
        }
        Ok(body)
    }

    fn json_rows(&self, sql: &str) -> AppResult<Vec<Vec<serde_json::Value>>> {
        let body = self.request(sql)?;
        if body.trim().is_empty() {
            return Ok(Vec::new());
        }
        let parsed: ChResponse =
            serde_json::from_str(&body).map_err(|e| AppError::query(e.to_string()))?;
        Ok(parsed.data)
    }
}

/// Minimal percent-encoding for a URL query value.
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn cell(v: &serde_json::Value) -> DbValue {
    match v {
        serde_json::Value::Null => DbValue::Null,
        serde_json::Value::Bool(b) => DbValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                DbValue::Int(i)
            } else {
                DbValue::Float(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        serde_json::Value::String(s) => DbValue::Text(s.clone()),
        other => DbValue::Text(other.to_string()),
    }
}

fn str_at(row: &[serde_json::Value], i: usize) -> String {
    row.get(i).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

impl DbConnection for ClickhouseConn {
    fn dialect(&self) -> Dialect {
        Dialect::Clickhouse
    }

    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_event: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        check_cancel(cancel)?;
        let body = self.request(sql)?;
        if body.trim().is_empty() {
            // DDL/DML — ClickHouse doesn't report affected counts over HTTP.
            return Ok(ExecMeta { columns: None, affected: None, stopped_early: false });
        }
        let parsed: Result<ChResponse, _> = serde_json::from_str(&body);
        match parsed {
            Ok(resp) => {
                let metas: Vec<ColumnMeta> = resp
                    .meta
                    .iter()
                    .map(|m| ColumnMeta::new(&m.name, &m.type_name))
                    .collect();
                let mut stopped_early = false;
                if let ControlFlow::Break(()) = on_event(RowEvent::Columns(metas.clone())) {
                    stopped_early = true;
                }
                for row in &resp.data {
                    check_cancel(cancel)?;
                    if stopped_early {
                        break;
                    }
                    let vals: Vec<DbValue> = row.iter().map(cell).collect();
                    if let ControlFlow::Break(()) = on_event(RowEvent::Row(vals)) {
                        stopped_early = true;
                    }
                }
                Ok(ExecMeta { columns: Some(metas), affected: None, stopped_early })
            }
            Err(_) => {
                // Statement carried its own FORMAT clause — present the raw
                // text as a single-column result.
                let metas = vec![ColumnMeta::new("result", "String")];
                let mut stopped_early = false;
                if let ControlFlow::Break(()) = on_event(RowEvent::Columns(metas.clone())) {
                    stopped_early = true;
                }
                for line in body.lines() {
                    if stopped_early {
                        break;
                    }
                    if let ControlFlow::Break(()) =
                        on_event(RowEvent::Row(vec![DbValue::Text(line.to_string())]))
                    {
                        stopped_early = true;
                    }
                }
                Ok(ExecMeta { columns: Some(metas), affected: None, stopped_early })
            }
        }
    }

    fn list_databases(&mut self) -> AppResult<Vec<String>> {
        let rows = self.json_rows(
            "SELECT name FROM system.databases WHERE name NOT IN ('system','INFORMATION_SCHEMA','information_schema') ORDER BY name",
        )?;
        Ok(rows.iter().map(|r| str_at(r, 0)).filter(|s| !s.is_empty()).collect())
    }

    fn catalog(&mut self) -> AppResult<Catalog> {
        use std::collections::BTreeMap;

        let hidden = "('system','INFORMATION_SCHEMA','information_schema')";
        let db_filter = if self.database.is_empty() {
            format!("database NOT IN {hidden}")
        } else {
            format!("database = '{}'", self.database.replace('\'', "\\'"))
        };

        let rels = self.json_rows(&format!(
            "SELECT database, name, engine, if(empty(primary_key), sorting_key, primary_key) \
             FROM system.tables WHERE {db_filter} ORDER BY database, name"
        ))?;

        let cols = self.json_rows(&format!(
            "SELECT database, table, name, type, default_expression \
             FROM system.columns WHERE {db_filter} ORDER BY database, table, position"
        ))
        .unwrap_or_default();

        let mut schemas: BTreeMap<String, BTreeMap<String, TableInfo>> = BTreeMap::new();
        let mut pk_of: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for row in &rels {
            let (db, name, engine) = (str_at(row, 0), str_at(row, 1), str_at(row, 2));
            let kind = if engine.contains("View") { "view" } else { "table" };
            let pk = str_at(row, 3);
            if !pk.is_empty() {
                pk_of.insert(
                    (db.clone(), name.clone()),
                    pk.split(',').map(|s| s.trim().to_string()).collect(),
                );
            }
            let mut info = TableInfo::new(name.clone(), kind);
            if kind == "table" && !engine.is_empty() {
                info.indexes.push(format!("engine: {engine}"));
            }
            schemas.entry(db).or_default().insert(name, info);
        }
        for row in &cols {
            let (db, tbl) = (str_at(row, 0), str_at(row, 1));
            if let Some(t) = schemas.get_mut(&db).and_then(|s| s.get_mut(&tbl)) {
                let ty = str_at(row, 3);
                let default = str_at(row, 4);
                t.columns.push(ColumnInfo {
                    name: str_at(row, 2),
                    nullable: ty.starts_with("Nullable("),
                    data_type: ty,
                    default: (!default.is_empty()).then_some(default),
                });
            }
        }
        for ((db, tbl), pk) in pk_of {
            if let Some(t) = schemas.get_mut(&db).and_then(|s| s.get_mut(&tbl)) {
                t.pk = pk;
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
        let q = Dialect::Clickhouse;
        let rows = self.json_rows(&format!(
            "SELECT create_table_query FROM system.tables WHERE database = '{}' AND name = '{}'",
            schema.replace('\'', "\\'"),
            table.replace('\'', "\\'")
        ))?;
        Ok(rows.first().map(|r| str_at(r, 0)).filter(|s| !s.is_empty()).or_else(|| {
            Some(format!(
                "-- no stored DDL for {}.{}",
                q.quote_ident(schema),
                q.quote_ident(table)
            ))
        }))
    }

    fn ping(&mut self) -> AppResult<()> {
        let body = self.request("SELECT 1")?;
        if body.contains('1') {
            Ok(())
        } else {
            Err(AppError::connect("ClickHouse: unexpected ping response"))
        }
    }
}
