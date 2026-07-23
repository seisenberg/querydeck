# Native Snowflake support — design & decisions (2026-07-12)

Status: **A merged upstream** — estie-inc/snowflake-connector-rs shipped its own cancellation API in
v1.1.0 (PR #206: `Session::query_handle` → `QueryHandle`/`QueryCanceller`, `ErrorKind::Cancelled`),
superseding our fork branch; QueryDeck depends on the crates.io release, no git dep needed.
**B implemented 2026-07-23** (`drivers/snowflake.rs`, all four auth methods; live verification
against a real account still pending). **C deferred.**

Goal: add a *native* Snowflake driver to QueryDeck (like postgres/mysql/mssql/clickhouse/trino),
so Snowflake works without the JDBC/JVM path. Snowflake has no simple wire protocol — it's an
HTTP/REST API — so "native" means wrapping a Rust client, not hand-rolling REST.

## Decision: wrap `snowflake-connector-rs`, don't hand-roll
`estie-inc/snowflake-connector-rs` v1.0.0, **MIT**, uses the **same stack we do** (reqwest 0.13 +
rustls + tokio 1.52). It supports all four auth methods we want: password+MFA, key-pair (JWT), OAuth
token, and external-browser SSO (crate marks the last experimental). Auth methods are behind cargo
features: `key-pair-auth`, `external-browser-sso`, `derive`.

It's async; we bridge to our blocking `DbConnection` trait the same way `mssql.rs` bridges tiberius:
each connection owns a current-thread `tokio::runtime::Runtime` and calls `rt.block_on(...)`.

Auth scope for v1 (user decision): **all four** methods in the connection form.

---

## A. Query cancellation — DONE (upstream contribution)
**Why it was needed:** Snowflake bills warehouse time. A user cancelling a long query must actually
stop execution, not just stop the client reading (which would keep billing). The crate exposed **no**
cancel path, **no** query id, and hid the session token (that hiding is just `Debug` redaction, not an
API stance).

**What we built** in `~/Documents/snowflake-connector-rs`, branch `feat/query-cancellation`
(commit `a45939a`; see `PR_DESCRIPTION.md` there — untracked):
- Public `CancellationToken` (clone-shared, `cancel()`/`is_cancelled()`), `tokio::sync::Notify` +
  `AtomicBool`, no new deps.
- `QueryOptions::with_cancellation_token`.
- On cancel: stop the submit+poll wait **and** `POST /queries/v1/abort-request` (the official
  connectors' cancel endpoint) so the warehouse stops.
- `ErrorKind::Cancelled` + `Error::is_cancelled()`.
- Tests green (399 lib tests), fmt + clippy clean.
- **Live-verified** against a real Snowflake account (external-browser SSO): a cancelled
  `SYSTEM$WAIT(120)` returns `is_cancelled()` in ~3s and shows stopped server-side in `QUERY_HISTORY`
  (`FAILED_WITH_ERROR`, error 604 "SQL execution canceled"). Harness: `~/Documents/snowflake-cancel-test`.
  Owner (Sam) is forking + opening the upstream PR.

**How QueryDeck will use it (in B):** each tab's `SnowflakeConn` keeps the token for its in-flight
query; `query_cancel` command calls `token.cancel()`. Implement `DbConnection::server_cancel` to return
a closure that cancels the token — no second session, no SQL `SYSTEM$CANCEL`.

Until the PR is released, QueryDeck depends on the fork/branch as a git dependency:
`snowflake-connector-rs = { git = "https://github.com/<fork>/snowflake-connector-rs", branch = "feat/query-cancellation", features = [...] }`, then switch to the crates.io version once merged.

---

## B. QueryDeck native driver — DESIGNED, TODO
New `drivers/snowflake.rs` implementing `DbConnection`, plus wiring.

Backend:
- `SnowflakeConn { rt: tokio::runtime::Runtime, session: snowflake_connector_rs::SnowflakeSession, params, cancel: Mutex<Option<CancellationToken>> }`.
- `open`: build `Client` with `ClientConfig::new(user, account, AuthConfig::…)` selected by
  `params.auth_method`, plus warehouse/database/schema/role; `create_session()` in `block_on`.
  External-browser opens the system browser here (user-initiated at connect).
- `execute`: mint a `CancellationToken`, stash a clone in `self.cancel`, run
  `session.query_with_options(sql, QueryOptions::new().with_cancellation_token(token))` in `block_on`,
  stream tables → `ColumnMeta` + `DbValue` (numeric→Int/Float, else Text, like `trino.rs::cell`),
  honour `max_rows` (`ControlFlow::Break`). Map `err.is_cancelled()` → `AppError::cancelled()`.
- `server_cancel`: return `Box::new(move || token.cancel())` from the stashed token (thread-safe).
- `catalog`: query `<db>.information_schema.tables`/`.columns` (Trino-shaped); no db → list databases.
- `list_databases`: `SHOW TERSE DATABASES`.
- `native_ddl`: `SELECT GET_DDL('TABLE', '<schema>.<table>')`.
- `ping`: `SELECT 1`.
- New `Dialect::Snowflake`: double-quoted idents; backslash+quote string escaping (Snowflake default);
  `TRUE/FALSE` booleans. Add to the `Dialect` match arms + `transfer.rs` type map.
- Wire into `DriverParams::Snowflake` + `connect()`.

Params / form (`SnowflakeParams`, `ConnectionModal`, `api.ts` DriverKind `"snowflake"`):
- `account`, `user`, `authMethod` (`password`|`keypair`|`external_browser`|`oauth`),
  `warehouse`, `database`, `schema`, `role`.
- Auth-conditional: password (+ optional MFA passcode); keypair → private-key PEM **file path** +
  optional passphrase; external_browser → none (label experimental); oauth → token.
- Add Snowflake to `ConnectionPicker` as a native built-in; map engine → existing Snowflake icon.

Secrets: `password_mut()` returns the active method's secret (password | passphrase | oauth token;
external_browser → none) for keychain storage. Private-key file stays on disk by path (like SSH-tunnel
keys). **MFA passcode is not persisted** (one-time TOTP) — usable only for the initial connect; for
hands-off reconnect use key-pair or a token. Password + *push* MFA works persistently.

Cargo: `snowflake-connector-rs = { git/version, features = ["key-pair-auth", "external-browser-sso"] }`
(+ `derive` only if we use typed rows; we use dynamic rows, so probably not).

Testing: Rust unit tests for `Dialect::Snowflake` + type mapping; frontend `tsc`/build + devmock form
smoke. **Live auth/query verification is the user's** (no Snowflake account here) — provide a per-method
checklist.

Out of scope (v1): PUT/GET file staging; streaming ingest; auto key-pair rotation (keys rotate
~60–90 days, org-managed, and we reference the PEM by path so a rotation is just swapping the file).

---

## C. SSO token persistence across restarts — DEFERRED
The crate hides the session token and can't reconstruct a session from one, and Snowflake session
tokens are short-lived with no "resume." True cross-restart SSO = **id-token caching** (request an
id_token during browser login, store in the OS keychain, reconnect via `authenticator=id_token`) — a
substantially larger upstream feature than A.

Decision: **not now.** For hands-off persistence use a long-lived token (Snowflake **PAT** / OAuth
token) stored in the keychain; external-browser stays interactive (re-prompts on restart / after the
~4h idle timeout). Revisit C as its own upstream PR if cached SSO is still wanted.
