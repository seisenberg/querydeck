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
| MySQL / MariaDB | native Rust (mysql crate, rustls TLS) | none |
| **ODBC** | thin binding loaded at runtime — connect to anything with an ODBC driver | Windows: built in · macOS: `brew install unixodbc` · Debian/Ubuntu: `apt install unixodbc` |
| **JDBC** | JNI bridge, JVM discovered at runtime — connect to anything with a JDBC driver | a Java 11+ runtime (e.g. [Temurin](https://adoptium.net)); set `JAVA_HOME` |

The **driver manager** (⚙ in the sidebar, or the status bar) shows ODBC/Java availability
and offers one-click downloads from Maven Central for common JDBC drivers: PostgreSQL,
MySQL, MariaDB, SQLite, DuckDB, ClickHouse, SQL Server, Oracle, H2, Trino. Downloaded
jars land in the app data directory and can be attached to a JDBC connection with one
click ("New connection"). Custom jars can be added to any JDBC connection.

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

## Notes & limitations (current state)

- Connection profiles (including passwords) are stored as plain JSON in the app config
  directory. OS keychain integration is a TODO.
- ODBC/JDBC values are read as text — fine for browsing/exporting, no binary fidelity.
- Cooperative cancel stops result streaming; PostgreSQL additionally cancels
  server-side. A long-running statement on other engines finishes server-side before the
  error surfaces.
- No SQL reformatting, VCS integration, or query history yet — deliberately out of scope
  for now.
