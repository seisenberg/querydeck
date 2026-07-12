//! Live tests against public endpoints. Network-dependent, so they are
//! #[ignore]d by default — run explicitly:
//!   cargo test --test live_endpoints -- --ignored --nocapture

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use querydeck_lib::testkit::{
    connect, ClickhouseParams, ConnectionProfile, DriverParams, RowEvent,
};

fn play_profile() -> ConnectionProfile {
    ConnectionProfile {
        id: "test-ch-play".into(),
        name: "ClickHouse play".into(),
        ssh_tunnel: None,
        params: DriverParams::Clickhouse(ClickhouseParams {
            host: "play.clickhouse.com".into(),
            port: 443,
            user: "explorer".into(),
            password: String::new(),
            database: String::new(),
            use_https: true,
            accept_invalid_certs: false,
        }),
    }
}

#[test]
#[ignore = "requires network access to play.clickhouse.com"]
fn clickhouse_play_query_and_catalog() {
    let mut conn = connect(&play_profile()).expect("connect to ClickHouse play");

    // Typed SELECT through the JSONCompact path.
    let cancel = AtomicBool::new(false);
    let mut cols: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut sink = |ev: RowEvent| -> ControlFlow<()> {
        match ev {
            RowEvent::Columns(c) => cols = c.iter().map(|m| m.name.clone()).collect(),
            RowEvent::Row(r) => rows.push(r.iter().map(|v| format!("{v:?}")).collect()),
        }
        ControlFlow::Continue(())
    };
    let meta = conn
        .execute(
            "SELECT 41 + 1 AS answer, 'hello' AS s, version() AS v",
            &cancel,
            &mut sink,
        )
        .expect("run select");
    assert!(meta.columns.is_some());
    assert_eq!(cols, vec!["answer", "s", "v"]);
    assert_eq!(rows.len(), 1);
    assert!(rows[0][0].contains("42"));
    println!("clickhouse play rows: {rows:?}");

    // Database switcher source.
    let dbs = conn.list_databases().expect("list databases");
    println!("clickhouse play databases: {dbs:?}");
    assert!(!dbs.is_empty());

    // Catalog introspection over system.tables/columns.
    let cat = conn.catalog().expect("catalog");
    println!(
        "clickhouse play catalog: {} schemas, first: {:?}",
        cat.schemas.len(),
        cat.schemas.first().map(|s| (&s.name, s.tables.len()))
    );
    assert!(!cat.schemas.is_empty());
    let with_cols = cat
        .schemas
        .iter()
        .flat_map(|s| &s.tables)
        .any(|t| !t.columns.is_empty());
    assert!(with_cols, "expected at least one table with columns");
}
