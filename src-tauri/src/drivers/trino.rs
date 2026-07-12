//! Trino, natively over its HTTP REST protocol (POST /v1/statement, then
//! follow nextUri pages). No JVM needed; also speaks to most Presto forks.

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde::Deserialize;

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, DbConnection, Dialect, ExecMeta, RowEvent, RowSink, TrinoParams};

pub struct TrinoConn {
    http: reqwest::blocking::Client,
    base: String,
    params: TrinoParams,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueryResults {
    id: String,
    next_uri: Option<String>,
    columns: Option<Vec<TrinoColumn>>,
    data: Option<Vec<Vec<serde_json::Value>>>,
    error: Option<TrinoError>,
    update_count: Option<u64>,
}

#[derive(Deserialize)]
struct TrinoColumn {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrinoError {
    message: String,
    error_name: Option<String>,
}

fn cell(v: &serde_json::Value) -> DbValue {
    match v {
        serde_json::Value::Null => DbValue::Null,
        serde_json::Value::Bool(b) => DbValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                DbValue::Int(i)
            } else {
                DbValue::float(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        serde_json::Value::String(s) => DbValue::Text(s.clone()),
        other => DbValue::Text(other.to_string()),
    }
}

impl TrinoConn {
    pub fn open(params: &TrinoParams) -> AppResult<Self> {
        let scheme = if params.use_https { "https" } else { "http" };
        let base = format!("{scheme}://{}:{}", params.host, params.port);
        let http = reqwest::blocking::Client::builder()
            .danger_accept_invalid_certs(params.accept_invalid_certs)
            .timeout(Duration::from_secs(600))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| AppError::connect(e.to_string()))?;
        Ok(Self { http, base, params: params.clone() })
    }

    fn auth(&self, req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        let mut req = req.header("X-Trino-User", &self.params.user);
        if !self.params.password.is_empty() {
            req = req.basic_auth(&self.params.user, Some(&self.params.password));
        }
        req
    }

    fn post_statement(&self, sql: &str) -> AppResult<QueryResults> {
        let mut req = self
            .auth(self.http.post(format!("{}/v1/statement", self.base)))
            .body(sql.to_string());
        if !self.params.catalog.is_empty() {
            req = req.header("X-Trino-Catalog", &self.params.catalog);
        }
        if !self.params.schema.is_empty() {
            req = req.header("X-Trino-Schema", &self.params.schema);
        }
        self.read(req)
    }

    fn read(&self, req: reqwest::blocking::RequestBuilder) -> AppResult<QueryResults> {
        let resp = req
            .send()
            .map_err(|e| AppError::connect(format!("Trino: {e}")))?;
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        if !status.is_success() {
            let detail = if body.trim().is_empty() { status.to_string() } else { body.trim().to_string() };
            return Err(AppError::query(format!("Trino: {detail}")));
        }
        serde_json::from_str(&body).map_err(|e| AppError::query(format!("Trino: {e}")))
    }

    /// Best-effort server-side kill for cancelled / row-limited queries.
    fn kill(&self, query_id: &str) {
        let _ = self
            .auth(self.http.delete(format!("{}/v1/query/{query_id}", self.base)))
            .send();
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

    fn quoted_catalog(&self) -> String {
        format!("\"{}\"", self.params.catalog.replace('"', "\"\""))
    }
}

impl DbConnection for TrinoConn {
    fn dialect(&self) -> Dialect {
        Dialect::Generic
    }

    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_event: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        check_cancel(cancel)?;
        let mut page = self.post_statement(sql)?;
        let query_id = page.id.clone();

        let mut columns: Option<Vec<ColumnMeta>> = None;
        let mut affected: Option<u64> = None;
        let mut stopped_early = false;

        loop {
            if let Some(err) = page.error {
                let name = err.error_name.map(|n| format!("{n}: ")).unwrap_or_default();
                return Err(AppError::query(format!("Trino: {name}{}", err.message)));
            }
            if columns.is_none() {
                if let Some(cols) = &page.columns {
                    let metas: Vec<ColumnMeta> = cols
                        .iter()
                        .map(|c| ColumnMeta::new(&c.name, &c.type_name))
                        .collect();
                    columns = Some(metas.clone());
                    if let ControlFlow::Break(()) = on_event(RowEvent::Columns(metas)) {
                        stopped_early = true;
                    }
                }
            }
            if let Some(count) = page.update_count {
                affected = Some(count);
            }
            for row in page.data.as_deref().unwrap_or_default() {
                if stopped_early {
                    break;
                }
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    self.kill(&query_id);
                    return Err(AppError::cancelled());
                }
                let vals: Vec<DbValue> = row.iter().map(cell).collect();
                if let ControlFlow::Break(()) = on_event(RowEvent::Row(vals)) {
                    stopped_early = true;
                }
            }
            let Some(next) = page.next_uri else { break };
            if stopped_early || cancel.load(std::sync::atomic::Ordering::Relaxed) {
                // Row limit hit or user cancel: stop the server-side query
                // instead of draining the rest of the result.
                self.kill(&query_id);
                if !stopped_early {
                    return Err(AppError::cancelled());
                }
                break;
            }
            // Empty in-progress pages come back fast while the query is
            // queued/planning; a short pause keeps this from busy-looping.
            if page.data.is_none() {
                std::thread::sleep(Duration::from_millis(50));
            }
            page = self.read(self.auth(self.http.get(&next)))?;
        }

        Ok(ExecMeta { columns, affected, stopped_early })
    }

    fn list_databases(&mut self) -> AppResult<Vec<String>> {
        // "Databases" map to Trino catalogs — the per-tab switcher swaps them.
        let rows = self.text_rows("SHOW CATALOGS")?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.into_iter().next().flatten())
            .filter(|c| c != "system")
            .collect())
    }

    fn catalog(&mut self) -> AppResult<Catalog> {
        use std::collections::BTreeMap;

        if self.params.catalog.is_empty() {
            // Without a catalog there is no schema to walk; the tree shows
            // the available catalogs instead (pick one on the connection).
            let cats = self.list_databases()?;
            return Ok(Catalog {
                schemas: cats
                    .into_iter()
                    .map(|c| SchemaInfo { name: format!("{c} (catalog — set it on the connection)"), tables: vec![] })
                    .collect(),
            });
        }

        let cat = self.quoted_catalog();
        let rels = self.text_rows(&format!(
            "SELECT table_schema, table_name, table_type FROM {cat}.information_schema.tables \
             WHERE table_schema <> 'information_schema' ORDER BY 1, 2"
        ))?;
        let cols = self
            .text_rows(&format!(
                "SELECT table_schema, table_name, column_name, data_type, is_nullable, column_default \
                 FROM {cat}.information_schema.columns \
                 WHERE table_schema <> 'information_schema' \
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

    fn ping(&mut self) -> AppResult<()> {
        let rows = self.text_rows("SELECT 1")?;
        if rows.is_empty() {
            return Err(AppError::connect("Trino: unexpected ping response"));
        }
        Ok(())
    }
}
