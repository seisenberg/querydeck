//! Microsoft SQL Server, natively over TDS via tiberius (rustls).

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use futures_util::StreamExt;
use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel, QueryItem};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, DbConnection, Dialect, ExecMeta, MssqlParams, RowEvent, RowSink};

pub struct MssqlConn {
    rt: tokio::runtime::Runtime,
    client: Client<Compat<TcpStream>>,
}

fn ms_err(e: tiberius::error::Error) -> AppError {
    AppError::query(format!("SQL Server: {e}"))
}

impl MssqlConn {
    pub fn open(params: &MssqlParams) -> AppResult<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| AppError::internal(e.to_string()))?;

        let mut config = Config::new();
        config.host(&params.host);
        config.port(params.port);
        config.authentication(AuthMethod::sql_server(&params.user, &params.password));
        if !params.database.is_empty() {
            config.database(&params.database);
        }
        config.encryption(if params.encrypt {
            EncryptionLevel::Required
        } else {
            EncryptionLevel::NotSupported
        });
        if params.accept_invalid_certs {
            config.trust_cert();
        }
        config.application_name("QueryDeck");

        let client = rt
            .block_on(async {
                let tcp = TcpStream::connect(config.get_addr()).await?;
                tcp.set_nodelay(true)?;
                Client::connect(config, tcp.compat_write()).await
            })
            .map_err(|e| AppError::connect(format!("SQL Server: {e}")))?;

        Ok(Self { rt, client })
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
                            other => Some(match other {
                                DbValue::Bool(b) => b.to_string(),
                                DbValue::Int(i) => i.to_string(),
                                DbValue::Float(f) => f.to_string(),
                                _ => unreachable!(),
                            }),
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

fn cell(data: ColumnData<'_>) -> DbValue {
    match data {
        ColumnData::U8(v) => v.map(|x| DbValue::Int(x as i64)).unwrap_or(DbValue::Null),
        ColumnData::I16(v) => v.map(|x| DbValue::Int(x as i64)).unwrap_or(DbValue::Null),
        ColumnData::I32(v) => v.map(|x| DbValue::Int(x as i64)).unwrap_or(DbValue::Null),
        ColumnData::I64(v) => v.map(DbValue::Int).unwrap_or(DbValue::Null),
        ColumnData::F32(v) => v.map(|x| DbValue::Float(x as f64)).unwrap_or(DbValue::Null),
        ColumnData::F64(v) => v.map(DbValue::Float).unwrap_or(DbValue::Null),
        ColumnData::Bit(v) => v.map(DbValue::Bool).unwrap_or(DbValue::Null),
        ColumnData::String(v) => v
            .map(|s| DbValue::Text(s.into_owned()))
            .unwrap_or(DbValue::Null),
        ColumnData::Guid(v) => v
            .map(|g| DbValue::Text(g.to_string()))
            .unwrap_or(DbValue::Null),
        ColumnData::Binary(v) => v
            .map(|b| DbValue::Text(format!("0x{}", hex(&b))))
            .unwrap_or(DbValue::Null),
        ColumnData::Numeric(v) => v
            .map(|n| DbValue::Text(n.to_string()))
            .unwrap_or(DbValue::Null),
        ColumnData::Xml(v) => v
            .map(|x| DbValue::Text(x.as_ref().to_string()))
            .unwrap_or(DbValue::Null),
        // Temporal values are decoded separately (typed try_get on the Row,
        // whose data is 'static); reaching here means NULL.
        ColumnData::DateTime(_)
        | ColumnData::SmallDateTime(_)
        | ColumnData::Time(_)
        | ColumnData::Date(_)
        | ColumnData::DateTime2(_)
        | ColumnData::DateTimeOffset(_) => DbValue::Null,
    }
}

/// Decode temporal columns to text via tiberius' chrono conversions; None
/// for non-temporal columns (handled by `cell`) and SQL NULLs.
fn temporal_text(row: &tiberius::Row, i: usize, ct: tiberius::ColumnType) -> Option<DbValue> {
    use tiberius::time::chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
    use tiberius::ColumnType as CT;
    match ct {
        CT::Daten => row
            .try_get::<NaiveDate, _>(i)
            .ok()
            .flatten()
            .map(|d| DbValue::Text(d.to_string())),
        CT::Timen => row
            .try_get::<NaiveTime, _>(i)
            .ok()
            .flatten()
            .map(|t| DbValue::Text(t.to_string())),
        CT::DatetimeOffsetn => row
            .try_get::<DateTime<FixedOffset>, _>(i)
            .ok()
            .flatten()
            .map(|t| DbValue::Text(t.to_rfc3339())),
        CT::Datetime | CT::Datetime2 | CT::Datetimen | CT::Datetime4 => row
            .try_get::<NaiveDateTime, _>(i)
            .ok()
            .flatten()
            .map(|t| DbValue::Text(t.format("%Y-%m-%d %H:%M:%S%.f").to_string())),
        _ => None,
    }
}

fn hex(b: &[u8]) -> String {
    let show = &b[..b.len().min(512)];
    let mut s = String::with_capacity(show.len() * 2);
    for byte in show {
        s.push_str(&format!("{byte:02x}"));
    }
    if b.len() > 512 {
        s.push('…');
    }
    s
}

/// Statements that produce result sets get streamed; everything else runs
/// through execute() so affected-row counts come back.
fn returns_rows(sql: &str) -> bool {
    let head = sql
        .trim_start()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    matches!(
        head.as_str(),
        "select" | "with" | "values" | "show" | "exec" | "execute" | "declare" | "sp_help"
    )
}

impl DbConnection for MssqlConn {
    fn dialect(&self) -> Dialect {
        Dialect::Mssql
    }

    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_event: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        let client = &mut self.client;
        let sql_owned = sql.to_string();

        if !returns_rows(sql) {
            let result = self
                .rt
                .block_on(client.execute(sql_owned.as_str(), &[]))
                .map_err(ms_err)?;
            let affected: u64 = result.rows_affected().iter().sum();
            return Ok(ExecMeta {
                columns: None,
                affected: Some(affected),
                stopped_early: false,
            });
        }

        self.rt.block_on(async move {
            let mut stream = client.simple_query(sql_owned).await.map_err(ms_err)?;
            let mut columns: Option<Vec<ColumnMeta>> = None;
            let mut stopped_early = false;

            while let Some(item) = stream.next().await {
                let item = item.map_err(ms_err)?;
                match item {
                    QueryItem::Metadata(meta) => {
                        if columns.is_none() {
                            let metas: Vec<ColumnMeta> = meta
                                .columns()
                                .iter()
                                .map(|c| ColumnMeta::new(c.name(), &format!("{:?}", c.column_type())))
                                .collect();
                            columns = Some(metas.clone());
                            if let ControlFlow::Break(()) = on_event(RowEvent::Columns(metas)) {
                                stopped_early = true;
                            }
                        }
                    }
                    QueryItem::Row(row) => {
                        check_cancel(cancel)?;
                        if stopped_early {
                            continue;
                        }
                        let temporal: Vec<Option<DbValue>> = row
                            .columns()
                            .iter()
                            .enumerate()
                            .map(|(i, c)| temporal_text(&row, i, c.column_type()))
                            .collect();
                        let vals: Vec<DbValue> = row
                            .into_iter()
                            .enumerate()
                            .map(|(i, d)| {
                                temporal.get(i).cloned().flatten().unwrap_or_else(|| cell(d))
                            })
                            .collect();
                        if let ControlFlow::Break(()) = on_event(RowEvent::Row(vals)) {
                            stopped_early = true;
                        }
                    }
                }
            }
            Ok(ExecMeta { columns, affected: None, stopped_early })
        })
    }

    fn list_databases(&mut self) -> AppResult<Vec<String>> {
        let rows = self.text_rows(
            "SELECT name FROM sys.databases WHERE HAS_DBACCESS(name) = 1 \
             AND name NOT IN ('model','tempdb') ORDER BY name",
        )?;
        Ok(rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect())
    }

    fn catalog(&mut self) -> AppResult<Catalog> {
        use std::collections::BTreeMap;

        let rels = self.text_rows(
            "SELECT s.name, o.name, CASE o.type WHEN 'V' THEN 'view' ELSE 'table' END \
             FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id \
             WHERE o.type IN ('U','V') ORDER BY s.name, o.name",
        )?;

        let cols = self.text_rows(
            "SELECT s.name, o.name, c.name, \
                    CONCAT(t.name, CASE \
                      WHEN t.name IN ('varchar','char','varbinary','binary') THEN \
                        CONCAT('(', IIF(c.max_length = -1, 'max', CAST(c.max_length AS varchar(12))), ')') \
                      WHEN t.name IN ('nvarchar','nchar') THEN \
                        CONCAT('(', IIF(c.max_length = -1, 'max', CAST(c.max_length / 2 AS varchar(12))), ')') \
                      WHEN t.name IN ('decimal','numeric') THEN \
                        CONCAT('(', c.precision, ',', c.scale, ')') \
                      ELSE '' END), \
                    c.is_nullable, object_definition(c.default_object_id) \
             FROM sys.columns c \
             JOIN sys.objects o ON o.object_id = c.object_id \
             JOIN sys.schemas s ON s.schema_id = o.schema_id \
             JOIN sys.types t ON t.user_type_id = c.user_type_id \
             WHERE o.type IN ('U','V') ORDER BY s.name, o.name, c.column_id",
        )
        .unwrap_or_default();

        let pks = self.text_rows(
            "SELECT s.name, o.name, c.name \
             FROM sys.index_columns ic \
             JOIN sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
             JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
             JOIN sys.objects o ON o.object_id = ic.object_id \
             JOIN sys.schemas s ON s.schema_id = o.schema_id \
             WHERE i.is_primary_key = 1 ORDER BY s.name, o.name, ic.key_ordinal",
        )
        .unwrap_or_default();

        let indexes = self.text_rows(
            "SELECT s.name, o.name, CONCAT(i.name, ' (', i.type_desc, \
                    IIF(i.is_unique = 1, ', unique', ''), ')') \
             FROM sys.indexes i \
             JOIN sys.objects o ON o.object_id = i.object_id \
             JOIN sys.schemas s ON s.schema_id = o.schema_id \
             WHERE o.type = 'U' AND i.name IS NOT NULL AND i.is_primary_key = 0 \
             ORDER BY s.name, o.name, i.name",
        )
        .unwrap_or_default();

        let fks = self.text_rows(
            "SELECT s.name, o.name, CONCAT('FOREIGN KEY (', pc.name, ') REFERENCES ', \
                    rs.name, '.', ro.name, ' (', rc.name, ')') \
             FROM sys.foreign_key_columns fkc \
             JOIN sys.objects o ON o.object_id = fkc.parent_object_id \
             JOIN sys.schemas s ON s.schema_id = o.schema_id \
             JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id \
             JOIN sys.objects ro ON ro.object_id = fkc.referenced_object_id \
             JOIN sys.schemas rs ON rs.schema_id = ro.schema_id \
             JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id",
        )
        .unwrap_or_default();

        let mut schemas: BTreeMap<String, BTreeMap<String, TableInfo>> = BTreeMap::new();
        let get = |row: &[Option<String>], i: usize| -> String {
            row.get(i).and_then(|v| v.clone()).unwrap_or_default()
        };
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
                let nullable = get(&row, 4);
                t.columns.push(ColumnInfo {
                    name: get(&row, 2),
                    data_type: get(&row, 3),
                    nullable: nullable == "true" || nullable == "1",
                    default: row.get(5).and_then(|v| v.clone()),
                });
            }
        }
        for row in pks {
            let (ns, tbl) = (get(&row, 0), get(&row, 1));
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                t.pk.push(get(&row, 2));
            }
        }
        for row in indexes {
            let (ns, tbl) = (get(&row, 0), get(&row, 1));
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                t.indexes.push(get(&row, 2));
            }
        }
        for row in fks {
            let (ns, tbl) = (get(&row, 0), get(&row, 1));
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                t.foreign_keys.push(get(&row, 2));
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
        // Views (and other modules) carry their full definition.
        let esc = |s: &str| s.replace('\'', "''");
        let rows = self.text_rows(&format!(
            "SELECT m.definition FROM sys.sql_modules m \
             JOIN sys.objects o ON o.object_id = m.object_id \
             JOIN sys.schemas s ON s.schema_id = o.schema_id \
             WHERE s.name = '{}' AND o.name = '{}'",
            esc(schema),
            esc(table)
        ))?;
        Ok(rows
            .first()
            .and_then(|r| r.first().cloned().flatten())
            .map(|d| d.trim().to_string()))
    }
}
