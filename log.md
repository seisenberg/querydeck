# Progress log

## Session 2026-07-11 (fresh context)
- Reviewed codebase; merged the new request items (keychain, connection list
  drag-reorder, delete popover, sidebar min-width/border fixes, JDBC catalog
  expansion, tree streamlining, live endpoint testing) into plan.md.
- Starting chunk 1: tab drag restore/animation + Open Console modal.

### Landed (commits)
1. Tab drag fix + FLIP; Open Console modal (always opens, saved consoles listed).
2. Sidebar polish: title collision fix (min width 240), rail border fix,
   connection drag-reorder (profiles_reorder cmd), delete confirm popover.
3. Reformatter: sql-formatter + Edit menu items (⌘⌥L), Formatter Settings modal,
   persistent config.json via config_save/config_load.
4. Find/replace panel (iI, .*, close button); autocomplete from query history
   (statements + words), Enter/Tab accept.
5. Lint: node-sql-parser DML validation + doubled-keyword squiggles; spellcheck
   with entropy/schema/keyword skips; heavy deps lazy-loaded (entry 944K).

Next: chunk 6 (native engines: mssql/tiberius, redshift, mariadb, clickhouse).

### Landed (continued)
6. Native engines: SQL Server (tiberius), Redshift (PG wire, tolerant catalog),
   MariaDB, ClickHouse (HTTP/JSONCompact). Dialects Mssql/Clickhouse added.
7. JDBC catalog: 26 verified latest vendor drivers; searchable manager;
   connection-modal preset picker.
8. Other objects tree: schema → kind grouping, collapsed by default;
   objects.rs fully tolerant per query.
9. Keychain: passwords → OS keychain w/ sentinel + migration + fallback.
10/11. Disk catalog cache + background refresh (catalog:updated), activity
   registry + statusbar icon (4 states) + Background Activity modal with
   cancel + Disconnect & clear cache kill switch.

Next: live testing in the real app (tab drag, modal, formatter, search,
lint, ClickHouse play endpoint, startup/shutdown timing).

### Verification pass (browser preview w/ mock backend + live tests)
- Open Console modal: sections + saved consoles ✓; tab drag: order changes,
  no stuck dim style ✓; per-connection numbering ✓.
- Lint: second SELECT red-squiggled, "custmers" dotted (spell), valid words
  clean ✓ (verified via cm-lintRange DOM classes).
- Autocomplete: history words + whole statements below caret; ArrowDown +
  Tab/Enter accept ✓.
- Reformat (⌘⌥L event): keywords uppercased, 2-space indent ✓; Formatter
  Settings modal with live preview ✓.
- Search panel: iI/.* toggles (regex found 5 matches for o\.\w+), close
  button at end, Esc closes ✓.
- Activity: icon idle→both states, "3 running" count, modal with QUERY/
  BACKGROUND badges, elapsed, kill switch ✓ (mock artifact fixed: mock must
  return fresh arrays like real IPC).
- Tree: Other objects → schema → kind grouping collapsed ✓; connection
  drag-reorder w/ click suppression ✓; delete popover w/ solid-red Delete ✓;
  sidebar min 240 ✓; collapsed rail clean (no stray border) ✓.
- ClickHouse NATIVE driver live vs play.clickhouse.com: SELECT (typed 42 /
  hello / 26.7.1.459), databases [blogs, default, git_clickhouse, mgbench],
  catalog 4 schemas ✓ (cargo test --test live_endpoints -- --ignored).
- Snowflake/Redshift: no public endpoints; user to test w/ private creds +
  NetSuite JDBC.
- Env limitation: screencapture can't capture window contents (no Screen
  Recording permission) and Apple Events denied — native-window driving is
  unavailable; browser preview + mock covers UI, Rust tests cover backend.

### Startup/shutdown (release build, measured)
- Spawn → profiles loaded: 0.75s wall (profile load itself 0.66ms).
- Spawn → backend setup complete (menus/window path): 1.12s wall (371ms in run()).
- SIGTERM → process exit: 18ms.
- Disk schema cache makes the tree instant on expand; background refresh
  heals staleness once per run. Entry JS chunk kept at 944K (parser +
  dictionary lazy-load). Conclusion: startup ~1.1–1.5s cold, shutdown
  instant; no further hot spots found.
