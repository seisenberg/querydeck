//! Non-table schema objects (functions, procedures, sequences, triggers,
//! roles) for the tree's "Other objects" section, plus their DDL.
//!
//! Everything goes through plain SQL on the existing connection, keyed off
//! the connection's dialect — so native drivers get full support and
//! ODBC/JDBC connections to engines with an information_schema still get a
//! best-effort routines list.

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use serde::Serialize;

use crate::drivers::{DbConnection, Dialect, RowEvent};
use crate::error::AppResult;
use crate::value::DbValue;

#[derive(Debug, Clone, Serialize)]
pub struct DbObject {
    /// Schema the object lives in; None for cluster-level objects (roles).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub name: String,
    /// "function" | "procedure" | "sequence" | "trigger" | "role"
    pub kind: String,
    /// Disambiguator / context: identity arguments for routines, the owning
    /// table for triggers, login/group for roles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn text(v: &DbValue) -> String {
    match v {
        DbValue::Null => String::new(),
        DbValue::Bool(b) => b.to_string(),
        DbValue::Int(i) => i.to_string(),
        DbValue::Float(f) => f.to_string(),
        DbValue::Text(s) => s.clone(),
    }
}

fn query(conn: &mut dyn DbConnection, sql: &str) -> AppResult<Vec<Vec<DbValue>>> {
    let cancel = AtomicBool::new(false);
    let mut rows: Vec<Vec<DbValue>> = Vec::new();
    let mut sink = |ev: RowEvent| -> ControlFlow<()> {
        if let RowEvent::Row(r) = ev {
            rows.push(r);
        }
        ControlFlow::Continue(())
    };
    conn.execute(sql, &cancel, &mut sink)?;
    Ok(rows)
}

fn lit(dialect: Dialect, s: &str) -> String {
    format!("'{}'", dialect.escape_str(s))
}

/// Tolerant query: introspection must degrade per-section, never blank the
/// whole "Other objects" list because one view is missing on this engine.
fn query_ok(conn: &mut dyn DbConnection, sql: &str) -> Vec<Vec<DbValue>> {
    query(conn, sql).unwrap_or_default()
}

pub fn list(conn: &mut dyn DbConnection) -> AppResult<Vec<DbObject>> {
    let dialect = conn.dialect();
    let mut out: Vec<DbObject> = Vec::new();

    match dialect {
        Dialect::Postgres => {
            for r in query_ok(
                conn,
                "SELECT n.nspname, p.proname, \
                        CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END, \
                        pg_get_function_identity_arguments(p.oid) \
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname NOT IN ('pg_catalog','information_schema') \
                   AND p.prokind IN ('f','p') ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: text(&r[2]),
                    detail: Some(text(&r[3])),
                });
            }
            for r in query_ok(
                conn,
                "SELECT schemaname, sequencename FROM pg_sequences ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: "sequence".into(),
                    detail: None,
                });
            }
            for r in query_ok(
                conn,
                "SELECT DISTINCT n.nspname, t.tgname, c.relname \
                 FROM pg_trigger t \
                 JOIN pg_class c ON c.oid = t.tgrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE NOT t.tgisinternal ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: "trigger".into(),
                    detail: Some(text(&r[2])),
                });
            }
            for r in query_ok(
                conn,
                "SELECT rolname, CASE WHEN rolcanlogin THEN 'login role' ELSE 'group role' END \
                 FROM pg_roles WHERE rolname NOT LIKE 'pg\\_%' ORDER BY 1",
            ) {
                out.push(DbObject {
                    schema: None,
                    name: text(&r[0]),
                    kind: "role".into(),
                    detail: Some(text(&r[1])),
                });
            }
        }
        Dialect::Mysql => {
            for r in query_ok(
                conn,
                "SELECT ROUTINE_SCHEMA, ROUTINE_NAME, LOWER(ROUTINE_TYPE) \
                 FROM information_schema.ROUTINES \
                 WHERE ROUTINE_SCHEMA = DATABASE() ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: text(&r[2]),
                    detail: None,
                });
            }
            for r in query_ok(
                conn,
                "SELECT TRIGGER_SCHEMA, TRIGGER_NAME, EVENT_OBJECT_TABLE \
                 FROM information_schema.TRIGGERS \
                 WHERE TRIGGER_SCHEMA = DATABASE() ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: "trigger".into(),
                    detail: Some(text(&r[2])),
                });
            }
        }
        Dialect::Sqlite => {
            for r in query_ok(
                conn,
                "SELECT name, tbl_name FROM sqlite_master WHERE type = 'trigger' ORDER BY 1",
            ) {
                out.push(DbObject {
                    schema: Some("main".into()),
                    name: text(&r[0]),
                    kind: "trigger".into(),
                    detail: Some(text(&r[1])),
                });
            }
        }
        Dialect::Mssql => {
            for r in query_ok(
                conn,
                "SELECT s.name, o.name, \
                        CASE WHEN o.type = 'P' THEN 'procedure' ELSE 'function' END \
                 FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id \
                 WHERE o.type IN ('P','FN','IF','TF') AND o.is_ms_shipped = 0 \
                 ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: text(&r[2]),
                    detail: None,
                });
            }
            for r in query_ok(
                conn,
                "SELECT s.name, sq.name FROM sys.sequences sq \
                 JOIN sys.schemas s ON s.schema_id = sq.schema_id ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: "sequence".into(),
                    detail: None,
                });
            }
            for r in query_ok(
                conn,
                "SELECT s.name, tr.name, o.name FROM sys.triggers tr \
                 JOIN sys.objects o ON o.object_id = tr.parent_id \
                 JOIN sys.schemas s ON s.schema_id = o.schema_id \
                 WHERE tr.is_ms_shipped = 0 ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: "trigger".into(),
                    detail: Some(text(&r[2])),
                });
            }
        }
        Dialect::Clickhouse => {
            for r in query_ok(
                conn,
                "SELECT name, if(origin = 'SQLUserDefined', 'user defined', origin) \
                 FROM system.functions WHERE origin != 'System' ORDER BY 1",
            ) {
                out.push(DbObject {
                    schema: None,
                    name: text(&r[0]),
                    kind: "function".into(),
                    detail: Some(text(&r[1])),
                });
            }
            for r in query_ok(
                conn,
                "SELECT database, name, status FROM system.dictionaries ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: "dictionary".into(),
                    detail: Some(text(&r[2])),
                });
            }
        }
        Dialect::Generic => {
            // Best effort: engines reachable over ODBC/JDBC that expose an
            // ANSI information_schema (Postgres, MySQL, SQL Server, …).
            for r in query_ok(
                conn,
                "SELECT routine_schema, routine_name, LOWER(routine_type) \
                 FROM information_schema.routines ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: text(&r[2]),
                    detail: None,
                });
            }
            for r in query_ok(
                conn,
                "SELECT trigger_schema, trigger_name, event_object_table \
                 FROM information_schema.triggers ORDER BY 1, 2",
            ) {
                out.push(DbObject {
                    schema: Some(text(&r[0])),
                    name: text(&r[1]),
                    kind: "trigger".into(),
                    detail: Some(text(&r[2])),
                });
            }
        }
    }
    Ok(out)
}

pub fn ddl(
    conn: &mut dyn DbConnection,
    kind: &str,
    schema: &str,
    name: &str,
    detail: &str,
) -> AppResult<String> {
    let d = conn.dialect();
    let not_available =
        || format!("-- DDL retrieval is not supported here ({kind} \"{name}\").\n");

    let ddl = match (d, kind) {
        (Dialect::Postgres, "function" | "procedure") => {
            let rows = query(
                conn,
                &format!(
                    "SELECT pg_get_functiondef(p.oid) \
                     FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                     WHERE n.nspname = {} AND p.proname = {} \
                       AND pg_get_function_identity_arguments(p.oid) = {}",
                    lit(d, schema),
                    lit(d, name),
                    lit(d, detail)
                ),
            )?;
            rows.first().map(|r| format!("{};", text(&r[0])))
        }
        (Dialect::Postgres, "sequence") => query(
            conn,
            &format!(
                "SELECT 'CREATE SEQUENCE ' || quote_ident(schemaname) || '.' || quote_ident(sequencename) \
                 || E'\\n  INCREMENT BY ' || increment_by \
                 || E'\\n  MINVALUE ' || min_value || E'\\n  MAXVALUE ' || max_value \
                 || E'\\n  START WITH ' || start_value || E'\\n  CACHE ' || cache_size \
                 || CASE WHEN cycle THEN E'\\n  CYCLE' ELSE '' END || ';' \
                 FROM pg_sequences WHERE schemaname = {} AND sequencename = {}",
                lit(d, schema),
                lit(d, name)
            ),
        )?
        .first()
        .map(|r| text(&r[0])),
        (Dialect::Postgres, "trigger") => query(
            conn,
            &format!(
                "SELECT pg_get_triggerdef(t.oid, true) \
                 FROM pg_trigger t \
                 JOIN pg_class c ON c.oid = t.tgrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = {} AND t.tgname = {} AND c.relname = {} \
                   AND NOT t.tgisinternal",
                lit(d, schema),
                lit(d, name),
                lit(d, detail)
            ),
        )?
        .first()
        .map(|r| format!("{};", text(&r[0]))),
        (Dialect::Postgres, "role") => query(
            conn,
            &format!(
                "SELECT 'CREATE ROLE ' || quote_ident(rolname) \
                 || CASE WHEN rolsuper THEN ' SUPERUSER' ELSE '' END \
                 || CASE WHEN rolcreatedb THEN ' CREATEDB' ELSE '' END \
                 || CASE WHEN rolcreaterole THEN ' CREATEROLE' ELSE '' END \
                 || CASE WHEN rolcanlogin THEN ' LOGIN' ELSE ' NOLOGIN' END \
                 || CASE WHEN rolreplication THEN ' REPLICATION' ELSE '' END \
                 || COALESCE(' CONNECTION LIMIT ' || NULLIF(rolconnlimit, -1), '') || ';' \
                 FROM pg_roles WHERE rolname = {}",
                lit(d, name)
            ),
        )?
        .first()
        .map(|r| text(&r[0])),
        (Dialect::Mysql, "function" | "procedure" | "trigger") => {
            let stmt = match kind {
                "function" => format!("SHOW CREATE FUNCTION {}", d.quote_ident(name)),
                "procedure" => format!("SHOW CREATE PROCEDURE {}", d.quote_ident(name)),
                _ => format!("SHOW CREATE TRIGGER {}", d.quote_ident(name)),
            };
            // The CREATE text is the longest text column of the result row.
            query(conn, &stmt)?.first().map(|r| {
                let mut best = String::new();
                for v in r {
                    let t = text(v);
                    if t.len() > best.len() {
                        best = t;
                    }
                }
                format!("{best};")
            })
        }
        (Dialect::Sqlite, "trigger") => query(
            conn,
            &format!(
                "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = {}",
                lit(d, name)
            ),
        )?
        .first()
        .map(|r| format!("{};", text(&r[0]))),
        (Dialect::Mssql, "function" | "procedure" | "trigger") => query(
            conn,
            &format!(
                "SELECT OBJECT_DEFINITION(OBJECT_ID({}))",
                lit(d, &format!("{schema}.{name}"))
            ),
        )?
        .first()
        .and_then(|r| {
            let t = text(&r[0]);
            (!t.is_empty()).then(|| t.trim().to_string())
        }),
        (Dialect::Mssql, "sequence") => query(
            conn,
            &format!(
                "SELECT CONCAT('CREATE SEQUENCE ', QUOTENAME(s.name), '.', QUOTENAME(sq.name), \
                        ' START WITH ', CAST(sq.start_value AS varchar(40)), \
                        ' INCREMENT BY ', CAST(sq.increment AS varchar(40)), ';') \
                 FROM sys.sequences sq JOIN sys.schemas s ON s.schema_id = sq.schema_id \
                 WHERE s.name = {} AND sq.name = {}",
                lit(d, schema),
                lit(d, name)
            ),
        )?
        .first()
        .map(|r| text(&r[0])),
        (Dialect::Clickhouse, "dictionary") => query(
            conn,
            &format!(
                "SELECT create_table_query FROM system.tables WHERE database = {} AND name = {}",
                lit(d, schema),
                lit(d, name)
            ),
        )?
        .first()
        .and_then(|r| {
            let t = text(&r[0]);
            (!t.is_empty()).then_some(t)
        }),
        (Dialect::Generic, "function" | "procedure") => query(
            conn,
            &format!(
                "SELECT routine_definition FROM information_schema.routines \
                 WHERE routine_schema = {} AND routine_name = {}",
                lit(d, schema),
                lit(d, name)
            ),
        )?
        .first()
        .and_then(|r| {
            let t = text(&r[0]);
            (!t.is_empty()).then(|| format!("-- routine body (information_schema)\n{t}"))
        }),
        _ => None,
    };

    Ok(ddl.unwrap_or_else(not_available))
}
