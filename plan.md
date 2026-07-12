# Work plan (remove items as they land; one commit per numbered chunk)

Ordered: quick UI wins first, then self-contained features, then driver work,
then perf/telemetry which touches everything. Progress notes in log.md.

## 1. UI fixes: tab drag + Open Console modal
- Fix dragged tab keeping its dimmed style after drop (no re-render on
  pointer-up) and add motion during drag: lifted style on the dragged tab,
  FLIP slide animation on displaced tabs.
- "+" / ⌘T always opens the picker modal (no silent auto-reopen).
- Retitle modal "Open Console — pick a connection"; add a section listing
  saved (closed) consoles so users can reopen them from here too.

## 2. Sidebar polish batch
- "Connections" header must not collide with the collapse/resize controls or
  the "+" button at narrow widths → raise the min sidebar width, drop old min.
- Fix stray border line drawn behind the minimize button when collapsed.
- Connections list: drag to reorder profiles, FLIP-animated; order persisted.
- ConnectionModal "Delete…" → popover warning ("consoles, queries and logs
  will be permanently removed") with a solid-red Delete that actually deletes.

## 3. Query reformatter (Edit menu)
- `sql-formatter` (npm) drives formatting; dialect auto-picked from driver.
- Edit menu gains "Reformat Query" (accelerator) + "Formatter Settings…".
- Settings modal: keyword case, indent width, lines between statements,
  dense operators; stored in persistent app config (new config_get/config_set
  commands writing config.json in the app data dir).

## 4. Editor: search + autocomplete polish
- Find/replace: regex `.*` and case `iI` toggle buttons in the console search
  panel, close button at the panel end; direct + regex search.
- Autocomplete: suggestions under the caret from schema (existing) plus query
  history; ArrowUp/Down navigate the list, Enter and Tab accept, mouse click
  accepts.

## 5. Editor: validation + spellcheck
- Lint pass with red squiggles from the SQL parse (e.g. second SELECT in
  "SELECT SELECT"), doubled-keyword heuristic.
- Spellcheck (dotted underline) over comments/strings/identifier words with
  typo heuristics that skip high-entropy tokens, digits, schema names and SQL
  keywords.

## 6. New engines, native: SQL Server + Redshift + MariaDB + ClickHouse
- MS SQL Server natively via tiberius (execute/catalog/list_databases/objects).
- Redshift as a first-class kind reusing the Postgres wire driver (port 5439,
  catalog/object queries tolerant of missing PG views).
- MariaDB as a first-class kind reusing the MySQL driver.
- ClickHouse natively over its HTTP interface (reqwest already in tree).
- Icons + engineOf mappings for the new kinds.

## 7. JDBC: Snowflake preset + DataGrip-scale driver catalog
- Registry entries (latest from vendor/Maven Central) approaching DataGrip's
  list: Snowflake, Oracle, ClickHouse, DuckDB, BigQuery, Databricks, Trino,
  Presto, Vertica, Exasol, H2, HSQLDB, Derby, Firebird, Informix, Teradata,
  SAP HANA, Cassandra, MongoDB, Redis (JDBC wrappers where sane), etc.
- Redesign DriverManager UI for the bigger list: search box, grouped/sorted,
  status column, one-click download of latest.
- Connection modal presets that prefill driver class + URL template.

## 8. Objects/tree: full native schema items + streamlined grouping
- Native engines return complete object sets (functions, procedures,
  sequences, triggers, views, materialized views, events, extensions…).
- "Other objects" grouped database → schema → kind, collapsed by default,
  for heavily populated business databases.
- objects.rs: each introspection query tolerated independently; generic
  dialect also tries information_schema.triggers; verify via ODBC/JDBC PG.

## 9. OS keychain integration for secrets
- `keyring` crate; passwords move out of profiles.json into macOS Keychain
  (and platform equivalents); migrate on first save/load; profiles.json keeps
  a sentinel.

## 10. Startup/shutdown speed + disk schema cache
- Measure cold start → usable window, and quit time; kill hot spots.
- Persist catalog cache to disk per profile; tree renders instantly from it
  on boot, background-refresh after a profile's first session connects.

## 11. Activity status icon + background activity modal
- Statusbar icon, four states: idle / user queries / background work / both.
- Click → modal listing running work; per-connection "Disconnect & clear
  cache" kill switch for stuck sessions.

## 12. Live testing + wrap-up
- Test what's reachable (local PG/MySQL/SQLite; public endpoints for
  ClickHouse play server; note Snowflake/Redshift need user's private creds —
  user offered to test those + NetSuite JDBC).
- README + memory notes; full build/test pass; final commit.
