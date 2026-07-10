//! Memory/throughput stress for the result store and execution path:
//! repeated large result sets must stay within the store's caps and the
//! process RSS must plateau rather than grow linearly.

use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use querydeck_lib::testkit::*;

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps failed");
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1024.0
}

#[test]
fn repeated_large_results_stay_bounded() {
    let dir = std::env::temp_dir().join("querydeck-stress");
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join(format!("stress-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);

    let profile = ConnectionProfile {
        id: "stress".into(),
        name: "stress".into(),
        ssh_tunnel: None,
        params: DriverParams::Sqlite(SqliteParams { path: db.to_string_lossy().into_owned() }),
    };

    let state = Arc::new(AppState::default());
    state.profiles.lock().push(profile);

    // Seed 200k rows with texty payloads (~50 bytes/row of strings).
    state
        .with_conn("stress", "seed", None, |conn| {
            let cancel = AtomicBool::new(false);
            let mut sink = |_e: RowEvent| ControlFlow::Continue(());
            conn.execute("CREATE TABLE t (id INTEGER, payload TEXT, tag TEXT)", &cancel, &mut sink)?;
            conn.execute(
                "WITH RECURSIVE g(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM g WHERE x < 200000) \
                 INSERT INTO t SELECT x, printf('payload-%d-%d', x, x*7), 'tag-' || (x % 50) FROM g",
                &cancel,
                &mut sink,
            )?;
            Ok(())
        })
        .expect("seed failed");

    let rss_start = rss_mb();
    let mut page_fetch_total_ms = 0.0;
    let mut exec_total_ms = 0.0;
    const ITERS: usize = 30;
    const MAX_ROWS: usize = 100_000;

    let mut result_ids: Vec<u64> = Vec::new();
    for i in 0..ITERS {
        let t0 = std::time::Instant::now();
        let mut columns: Vec<ColumnMeta> = Vec::new();
        let mut rows: Vec<Vec<DbValue>> = Vec::new();
        state
            .with_conn("stress", "tab1", None, |conn| {
                let cancel = AtomicBool::new(false);
                let mut sink = |e: RowEvent| match e {
                    RowEvent::Columns(c) => {
                        columns = c;
                        ControlFlow::Continue(())
                    }
                    RowEvent::Row(r) => {
                        rows.push(r);
                        if rows.len() >= MAX_ROWS { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
                    }
                };
                conn.execute("SELECT id, payload, tag FROM t", &cancel, &mut sink)
            })
            .expect("query failed");
        exec_total_ms += t0.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(rows.len(), MAX_ROWS);

        let id = state.store_result(columns, rows);
        result_ids.push(id);

        // Page through like the grid does (5 pages of 500).
        let t1 = std::time::Instant::now();
        for p in 0..5 {
            let _page = state
                .with_result(id, |r| r.rows[p * 500..(p + 1) * 500].to_vec())
                .expect("page fetch failed");
        }
        page_fetch_total_ms += t1.elapsed().as_secs_f64() * 1000.0;

        // The frontend frees a tab's previous result on re-run; emulate that
        // for every other run so eviction ALSO gets exercised (odd runs pile
        // up until the caps kick in).
        if i % 2 == 0 {
            if result_ids.len() >= 2 {
                let old = result_ids.remove(0);
                state.free_results(&[old]);
            }
        }
    }

    let (results, total_rows, total_bytes) = state.store_stats();
    let rss_end = rss_mb();
    println!(
        "stress: {ITERS} iters × {MAX_ROWS} rows · store: {results} results, {total_rows} rows, {:.1} MB approx · exec avg {:.0} ms · 5-page fetch avg {:.2} ms · RSS {:.0} → {:.0} MB",
        total_bytes as f64 / 1e6,
        exec_total_ms / ITERS as f64,
        page_fetch_total_ms / ITERS as f64,
        rss_start,
        rss_end,
    );

    // Caps hold: at 100k rows × ~100 B/row each result is ~10-13 MB, so the
    // byte cap (256 MB) bounds the store well below the uncapped total
    // (30 × 13 MB ≈ 390 MB).
    assert!(total_bytes <= 300 * 1024 * 1024, "store bytes exceeded cap: {total_bytes}");
    assert!(total_rows <= 2_000_000, "store rows exceeded cap: {total_rows}");
    // RSS must not grow linearly with iterations (i.e. far below the
    // uncapped ~400 MB of retained results + allocator overhead).
    assert!(
        rss_end - rss_start < 450.0,
        "RSS grew suspiciously: {rss_start:.0} MB -> {rss_end:.0} MB"
    );

    state.close_session("stress", "tab1");
    state.close_session("stress", "seed");
    let _ = std::fs::remove_file(&db);
}
