//! End-to-end test of the native DuckDB driver (in-memory database).

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use querydeck_lib::testkit::{connect, ConnectionProfile, DriverParams, DuckdbParams, RowEvent};

fn mem_profile() -> ConnectionProfile {
    ConnectionProfile {
        id: "test-duckdb".into(),
        name: "duckdb mem".into(),
        ssh_tunnel: None,
        params: DriverParams::Duckdb(DuckdbParams { path: String::new() }),
    }
}

fn run(conn: &mut Box<dyn querydeck_lib::testkit::DbConnection>, sql: &str) -> Vec<Vec<String>> {
    let cancel = AtomicBool::new(false);
    let mut rows = Vec::new();
    let mut sink = |ev: RowEvent| -> ControlFlow<()> {
        if let RowEvent::Row(r) = ev {
            rows.push(r.iter().map(|v| format!("{v:?}")).collect());
        }
        ControlFlow::Continue(())
    };
    conn.execute(sql, &cancel, &mut sink).expect(sql);
    rows
}

#[test]
fn duckdb_execute_catalog_ddl() {
    let mut conn = connect(&mem_profile()).expect("open in-memory duckdb");

    // DDL + DML through the affected-count path.
    run(&mut conn, "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, score DOUBLE)");
    let cancel = AtomicBool::new(false);
    let mut sink = |_ev: RowEvent| ControlFlow::Continue(());
    let meta = conn
        .execute(
            "INSERT INTO people VALUES (1, 'ada', 9.5), (2, 'grace', 9.9)",
            &cancel,
            &mut sink,
        )
        .expect("insert");
    assert_eq!(meta.affected, Some(2));

    // Typed SELECT (ints, floats, text, aggregate) incl. FROM-first syntax.
    let rows = run(&mut conn, "SELECT id, name, score FROM people ORDER BY id");
    assert_eq!(rows.len(), 2);
    assert!(rows[0][1].contains("ada"));
    let agg = run(&mut conn, "FROM people SELECT count(*), avg(score)");
    assert_eq!(agg.len(), 1);
    assert!(agg[0][0].contains('2'));

    // Catalog: table with columns + pk.
    let cat = conn.catalog().expect("catalog");
    let table = cat
        .schemas
        .iter()
        .flat_map(|s| &s.tables)
        .find(|t| t.name == "people")
        .expect("people table in catalog");
    assert_eq!(table.columns.len(), 3);
    assert_eq!(table.pk, vec!["id".to_string()]);

    // Native DDL from duckdb_tables().
    let ddl = conn.native_ddl("main", "people").expect("ddl").expect("some ddl");
    assert!(ddl.to_lowercase().contains("create table"), "{ddl}");
}
