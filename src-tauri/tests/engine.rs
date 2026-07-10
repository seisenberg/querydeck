//! End-to-end engine tests against a temp SQLite database: execution with
//! row limits, catalog introspection, native DDL, and the SQL-literal batch
//! insert path used by imports.

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use querydeck_lib::testkit::*;

fn tmp_db(name: &str) -> String {
    let dir = std::env::temp_dir().join("querydeck-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

fn exec_collect(
    conn: &mut Box<dyn DbConnection>,
    sql: &str,
    max_rows: usize,
) -> (Option<Vec<ColumnMeta>>, Vec<Vec<DbValue>>, bool) {
    let cancel = AtomicBool::new(false);
    let mut cols = None;
    let mut rows: Vec<Vec<DbValue>> = Vec::new();
    let mut sink = |ev: RowEvent| match ev {
        RowEvent::Columns(c) => {
            cols = Some(c);
            ControlFlow::Continue(())
        }
        RowEvent::Row(r) => {
            rows.push(r);
            if max_rows > 0 && rows.len() >= max_rows {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    };
    let meta = conn.execute(sql, &cancel, &mut sink).expect("execute failed");
    (cols, rows, meta.stopped_early)
}

fn open_sqlite(path: &str) -> Box<dyn DbConnection> {
    let profile = ConnectionProfile {
        id: "test".into(),
        name: "test".into(),
        ssh_tunnel: None,
        params: DriverParams::Sqlite(SqliteParams { path: path.into() }),
    };
    connect(&profile).expect("connect failed")
}

#[test]
fn sqlite_roundtrip_with_limit() {
    let path = tmp_db("roundtrip");
    let mut conn = open_sqlite(&path);

    exec_collect(&mut conn, "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, score REAL)", 0);
    // seed 1000 rows
    let mut values = Vec::new();
    for i in 0..1000 {
        values.push(format!("({i}, 'name-{i}', {i}.5)"));
    }
    let (_, _, _) = exec_collect(
        &mut conn,
        &format!("INSERT INTO t (id, name, score) VALUES {}", values.join(",")),
        0,
    );

    // limited fetch stops early
    let (cols, rows, stopped) = exec_collect(&mut conn, "SELECT * FROM t ORDER BY id", 100);
    let cols = cols.expect("no columns");
    assert_eq!(cols.len(), 3);
    assert_eq!(cols[0].name, "id");
    assert_eq!(rows.len(), 100);
    assert!(stopped, "expected early stop at limit");

    // unlimited fetch gets everything
    let (_, rows, stopped) = exec_collect(&mut conn, "SELECT * FROM t", 0);
    assert_eq!(rows.len(), 1000);
    assert!(!stopped);

    // DML reports affected rows
    let cancel = AtomicBool::new(false);
    let mut sink = |_ev: RowEvent| ControlFlow::Continue(());
    let meta = conn
        .execute("UPDATE t SET score = 0 WHERE id < 10", &cancel, &mut sink)
        .unwrap();
    assert_eq!(meta.affected, Some(10));
}

#[test]
fn sqlite_catalog_and_ddl() {
    let path = tmp_db("catalog");
    let mut conn = open_sqlite(&path);
    exec_collect(
        &mut conn,
        "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL, age INT DEFAULT 7)",
        0,
    );
    exec_collect(&mut conn, "CREATE VIEW adults AS SELECT * FROM users WHERE age >= 18", 0);

    let cat = conn.catalog().expect("catalog failed");
    assert_eq!(cat.schemas.len(), 1);
    let tables = &cat.schemas[0].tables;
    let users = tables.iter().find(|t| t.name == "users").expect("users missing");
    assert_eq!(users.kind, "table");
    assert_eq!(users.pk, vec!["id".to_string()]);
    let email = users.columns.iter().find(|c| c.name == "email").unwrap();
    assert!(!email.nullable);
    let age = users.columns.iter().find(|c| c.name == "age").unwrap();
    assert_eq!(age.default.as_deref(), Some("7"));
    assert!(tables.iter().any(|t| t.name == "adults" && t.kind == "view"));

    let ddl = conn.native_ddl("main", "users").unwrap().expect("no ddl");
    assert!(ddl.to_uppercase().contains("CREATE TABLE"), "ddl: {ddl}");

    // generated DDL path also works
    let generated = generate_table_ddl(Dialect::Sqlite, "main", users);
    assert!(generated.contains("PRIMARY KEY"), "generated: {generated}");
}

#[test]
fn sql_literal_escaping() {
    let d = Dialect::Sqlite;
    assert_eq!(d.literal(&DbValue::Text("it's".into())), "'it''s'");
    assert_eq!(d.literal(&DbValue::Null), "NULL");
    let m = Dialect::Mysql;
    assert_eq!(m.literal(&DbValue::Text(r"a\b".into())), r"'a\\b'");
    assert_eq!(m.quote_ident("we`ird"), "`we``ird`");
}

#[test]
fn statement_splitting_matches_engine() {
    let stmts = split_statements(
        "CREATE TABLE x(a INT); INSERT INTO x VALUES (1); -- done;\nSELECT * FROM x",
    );
    assert_eq!(stmts.len(), 3);

    let path = tmp_db("split");
    let mut conn = open_sqlite(&path);
    for s in &stmts {
        exec_collect(&mut conn, &s.sql, 0);
    }
    let (_, rows, _) = exec_collect(&mut conn, "SELECT count(*) FROM x", 0);
    match rows[0][0] {
        DbValue::Int(n) => assert_eq!(n, 1),
        ref other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn cancel_flag_stops_execution() {
    let path = tmp_db("cancel");
    let mut conn = open_sqlite(&path);
    exec_collect(&mut conn, "CREATE TABLE big (n INTEGER)", 0);
    let mut values = Vec::new();
    for i in 0..5000 {
        values.push(format!("({i})"));
    }
    exec_collect(&mut conn, &format!("INSERT INTO big VALUES {}", values.join(",")), 0);

    let cancel = AtomicBool::new(false);
    let mut seen = 0usize;
    let mut sink = |ev: RowEvent| {
        if let RowEvent::Row(_) = ev {
            seen += 1;
            if seen == 50 {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        ControlFlow::Continue(())
    };
    let err = conn
        .execute("SELECT * FROM big", &cancel, &mut sink)
        .expect_err("expected cancellation error");
    assert_eq!(err.kind, ErrorKind::Cancelled);
    assert!(seen >= 50 && seen < 5000);
}
