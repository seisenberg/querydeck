# QueryDeck

A lightweight, cross-platform SQL IDE (think "DataGrip lite") built with **Tauri 2**, **React**, and **Rust**.
Runs on macOS, Windows, and Linux.

![status](https://img.shields.io/badge/status-early-lightgrey)

## What it does

- **SQL console with tabs** — every tab has its own physical database connection, so long
  queries in one tab never block another. `⌘⏎` runs the statement under the cursor (or the
  selection), `⌘⇧⏎` runs the whole script. Scripts are split respecting strings, comments,
  `$dollar$` quoting, and backticks.
- **Results grid built for large data** — rows stream out of the driver into a compact
  backend store; the grid virtualizes rendering and fetches pages on demand, so a
  100k-row result scrolls smoothly. An automatic row limit (dropdown: 100 / 500 / 1k /
  10k / 100k / no limit) protects against accidental `SELECT * FROM huge`, and the UI
  tells you when a result was truncated. Queries are cancellable (server-side cancel on
  PostgreSQL, cooperative elsewhere).
- **Schema navigator** — connections → schemas → tables/views → columns (with types, PK
  markers). Double-click a table to open its data in a new tab. Right-click for DDL,
  export, and import actions.
- **DDL viewer/generator** — native engine DDL where available (`SHOW CREATE TABLE`,
  `sqlite_master`, `pg_get_viewdef`), otherwise generated from catalog metadata
  (columns, defaults, PK, FKs, indexes).
- **Import wizard** (DataGrip-style) — pick a CSV/TSV, delimiter + header sniffed
  automatically, map file columns to table columns (auto-matched by name), create a new
  table (with inferred column types you can edit) or insert into an existing one.
  Batched, transactional, with live progress and cancel.
- **Export** — stream any query or table to CSV / TSV / JSON / SQL INSERTs. Exports
  re-run the query and stream straight from the database cursor to disk, so they are not
  limited by what the grid loaded.

## Drivers

| Driver | How it works | Setup |
| --- | --- | --- |
| SQLite | bundled (rusqlite) | none |
| PostgreSQL | native Rust (tokio-postgres + rustls TLS) | none |
| Amazon Redshift | native (PG wire protocol, Redshift-tolerant introspection) | none |
| MySQL | native Rust (mysql crate, rustls TLS) | none |
| MariaDB | native (MySQL wire protocol) | none |
| SQL Server | native Rust (tiberius, rustls TLS) | none |
| ClickHouse | native (HTTP interface, typed JSON results) | none |
| **ODBC** | thin binding loaded at runtime — connect to anything with an ODBC driver | Windows: built in · macOS: `brew install unixodbc` · Debian/Ubuntu: `apt install unixodbc` |
| **JDBC** | JNI bridge, JVM discovered at runtime — connect to anything with a JDBC driver | a Java 11+ runtime (e.g. [Temurin](https://adoptium.net)); set `JAVA_HOME` |

The **driver manager** (File → Database Drivers…, or the status bar) shows ODBC/Java
availability and offers one-click downloads of the latest vendor JDBC drivers from
Maven Central — 26 engines including Snowflake, Oracle, Db2, Redshift, Athena,
Teradata, Vertica, Exasol, SAP HANA, Informix, Hive/Spark, Trino, Presto, Firebird,
HSQLDB, Derby, MongoDB (Atlas SQL) and Sybase. The list is searchable; downloaded jars
can start a prefilled JDBC connection with one click, and the JDBC form has a preset
picker for URL template + driver class. Custom jars can be added to any connection.

Because the ODBC driver manager and the JVM are loaded dynamically, the app starts and
works fine on machines that have neither — those connection types just explain what to
install.

## Development

Prereqs: Rust (stable), Node 20+.

```bash
npm install
npm run tauri dev     # run the app
npm run tauri build   # produce installers/bundles for the current OS

cd src-tauri && cargo test   # engine tests (SQLite end-to-end)
npx tsc --noEmit             # typecheck the frontend
```

### Architecture

```
src/                    React UI (tabs, editor, grid, tree, wizards)
  api.ts                typed wrappers over tauri invoke
  store.ts              zustand app state
  components/           SqlEditor (CodeMirror 6), ResultsGrid (virtualized), …
src-tauri/src/
  drivers/              DbConnection trait + sqlite / postgres / mysql / odbc / jdbc
  sqlsplit.rs           statement splitter (strings, comments, $tag$, backticks)
  state.rs              sessions (one per profile+tab), result store, cancel flags
  catalog.rs, ddl.rs    introspection model + DDL generation
  transfer.rs           CSV import (mapping, batches, progress) and streaming export
  registry.rs           JDBC driver packages + Maven Central downloads
```

Row values cross the IPC boundary as plain JSON (null/bool/number/string); result sets
live in the Rust side (capped at ~2M rows total, oldest evicted) and the grid pages
through them with `page_fetch`.

## Editor & workflow niceties

- **Reformat Query** (Edit menu, `⌘⌥L`) via sql-formatter, dialect-aware; Formatter
  Settings (keyword case, indent, blank lines, dense operators) persist in the app
  config and preview live.
- **Find/replace** with case-insensitive (`iI`) and regex (`.*`) toggles.
- **Autocomplete** from the schema *and* your query history (recent statements +
  identifier vocabulary); arrows navigate, Enter/Tab/click accept.
- **Validation**: DML statements parse in the connection's dialect (red squiggles,
  e.g. the second `SELECT` in `SELECT SELECT`); doubled-keyword heuristic everywhere.
- **Spellcheck** over comments/strings/identifiers (dotted underline) that skips
  high-entropy tokens, acronyms, SQL vocabulary and schema names.
- **Schema cache on disk** — the tree renders instantly on launch from the last known
  schema, then refreshes in the background once per run.
- **Activity icon** in the status bar (idle / queries / background / both); click it
  for the Background Activity modal with per-query cancel and a per-connection
  "Disconnect & clear cache" kill switch.
- **OS keychain** — connection passwords live in the macOS Keychain / Windows
  Credential Manager / Secret Service; `connections.json` only holds a sentinel
  (existing plaintext profiles migrate automatically).

## Notes & limitations (current state)

- ODBC/JDBC values are read as text — fine for browsing/exporting, no binary fidelity.
- Cooperative cancel stops result streaming; PostgreSQL additionally cancels
  server-side. A long-running statement on other engines finishes server-side before the
  error surfaces.
- ODBC connection strings may embed `Pwd=`; those stay in the connection string (the
  keychain covers the discrete password fields of the other drivers).
- No VCS integration yet.
