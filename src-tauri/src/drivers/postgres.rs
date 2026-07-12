use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use futures_util::StreamExt;
use tokio_postgres::config::SslMode;
use tokio_postgres::{CancelToken, Client, Config, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, DbConnection, Dialect, ExecMeta, PostgresParams, RowEvent, RowSink};

pub struct PostgresConn {
    rt: tokio::runtime::Runtime,
    client: Client,
    cancel_token: CancelToken,
    tls: MakeRustlsConnect,
    /// Redshift speaks the PG wire protocol but is PG-8-era: some catalog
    /// views/functions are missing, so introspection degrades gracefully.
    redshift: bool,
}

/// rustls verifier that accepts any server certificate (opt-in, for dev DBs
/// with self-signed certs).
#[derive(Debug)]
struct NoVerify(rustls::crypto::WebPkiSupportedAlgorithms);

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.supported_schemes()
    }
}

fn make_tls(accept_invalid: bool) -> AppResult<MakeRustlsConnect> {
    let provider = rustls::crypto::CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));

    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let config = if accept_invalid {
        let algs = provider.signature_verification_algorithms;
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| AppError::connect(e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(algs)))
            .with_no_client_auth()
    } else {
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| AppError::connect(e.to_string()))?
            .with_root_certificates(roots)
            .with_no_client_auth()
    };
    Ok(MakeRustlsConnect::new(config))
}

impl PostgresConn {
    pub fn open(params: &PostgresParams) -> AppResult<Self> {
        Self::open_inner(params, false)
    }

    pub fn open_redshift(params: &PostgresParams) -> AppResult<Self> {
        Self::open_inner(params, true)
    }

    fn open_inner(params: &PostgresParams, redshift: bool) -> AppResult<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| AppError::internal(e.to_string()))?;

        let mut config = Config::new();
        config
            .host(&params.host)
            .port(params.port)
            .user(&params.user)
            .dbname(&params.database)
            .application_name("QueryDeck");
        if !params.password.is_empty() {
            config.password(&params.password);
        }
        config.ssl_mode(match params.ssl_mode.as_str() {
            "disable" => SslMode::Disable,
            "require" => SslMode::Require,
            _ => SslMode::Prefer,
        });

        let tls = make_tls(params.accept_invalid_certs)?;
        let (client, connection) = rt
            .block_on(config.connect(tls.clone()))
            .map_err(|e| AppError::connect(format!("PostgreSQL: {e}")))?;
        // Drive the connection as a background task on this runtime; it gets
        // polled whenever we block_on.
        rt.spawn(async move {
            let _ = connection.await;
        });

        let cancel_token = client.cancel_token();
        Ok(Self { rt, client, cancel_token, tls, redshift })
    }

    fn simple_rows(&mut self, sql: &str) -> AppResult<Vec<Vec<Option<String>>>> {
        let msgs = self
            .rt
            .block_on(self.client.simple_query(sql))
            .map_err(|e| AppError::query(pg_err(e)))?;
        let mut rows = Vec::new();
        for m in msgs {
            if let SimpleQueryMessage::Row(r) = m {
                let mut row = Vec::with_capacity(r.len());
                for i in 0..r.len() {
                    row.push(r.get(i).map(|s| s.to_string()));
                }
                rows.push(row);
            }
        }
        Ok(rows)
    }

    /// Best-effort variant for introspection queries that may not exist on
    /// every PG-compatible engine (Redshift, forks, restricted users).
    fn simple_rows_or_empty(&mut self, sql: &str) -> Vec<Vec<Option<String>>> {
        self.simple_rows(sql).unwrap_or_default()
    }

    /// A cancel handle usable from another thread.
    pub fn cancel_handle(&self) -> PgCancelHandle {
        PgCancelHandle { token: self.cancel_token.clone(), tls: self.tls.clone() }
    }
}

pub struct PgCancelHandle {
    token: CancelToken,
    tls: MakeRustlsConnect,
}

impl PgCancelHandle {
    pub fn cancel(&self) {
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
            let _ = rt.block_on(self.token.cancel_query(self.tls.clone()));
        }
    }
}

fn pg_err(e: tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => {
            let mut s = format!("{}: {}", db.code().code(), db.message());
            if let Some(d) = db.detail() {
                s.push_str(&format!("\n{d}"));
            }
            if let Some(h) = db.hint() {
                s.push_str(&format!("\nhint: {h}"));
            }
            s
        }
        None => e.to_string(),
    }
}

impl DbConnection for PostgresConn {
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    fn server_cancel(&self) -> Option<Box<dyn Fn() + Send + Sync>> {
        let handle = self.cancel_handle();
        Some(Box::new(move || handle.cancel()))
    }

    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_row: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        let client = &self.client;
        let sql = sql.to_string();
        self.rt.block_on(async move {
            let stream = client
                .simple_query_raw(&sql)
                .await
                .map_err(|e| AppError::query(pg_err(e)))?;
            futures_util::pin_mut!(stream);

            let mut columns: Option<Vec<ColumnMeta>> = None;
            let mut affected: Option<u64> = None;
            let mut stopped_early = false;

            while let Some(msg) = stream.next().await {
                let msg = msg.map_err(|e| AppError::query(pg_err(e)))?;
                match msg {
                    SimpleQueryMessage::RowDescription(cols) => {
                        if columns.is_none() {
                            let metas: Vec<ColumnMeta> =
                                cols.iter().map(|c| ColumnMeta::new(c.name(), "")).collect();
                            columns = Some(metas.clone());
                            if let ControlFlow::Break(()) = on_row(RowEvent::Columns(metas)) {
                                stopped_early = true;
                            }
                        }
                    }
                    SimpleQueryMessage::Row(r) => {
                        check_cancel(cancel)?;
                        if stopped_early {
                            continue; // drain remainder
                        }
                        let mut row = Vec::with_capacity(r.len());
                        for i in 0..r.len() {
                            row.push(match r.get(i) {
                                None => DbValue::Null,
                                Some(s) => DbValue::Text(s.to_string()),
                            });
                        }
                        if let ControlFlow::Break(()) = on_row(RowEvent::Row(row)) {
                            stopped_early = true;
                        }
                    }
                    SimpleQueryMessage::CommandComplete(n) => {
                        if columns.is_none() {
                            affected = Some(n);
                        }
                    }
                    _ => {}
                }
            }
            Ok(ExecMeta { columns, affected, stopped_early })
        })
    }

    fn list_databases(&mut self) -> AppResult<Vec<String>> {
        let rows = self.simple_rows(
            "SELECT datname FROM pg_database \
             WHERE NOT datistemplate AND has_database_privilege(datname, 'CONNECT') \
             ORDER BY datname",
        )?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.into_iter().next().flatten())
            .collect())
    }

    fn catalog(&mut self) -> AppResult<Catalog> {
        const HIDDEN: &str = "('pg_catalog','information_schema','pg_toast')";

        let rels = self.simple_rows(&format!(
            "SELECT n.nspname, c.relname, c.relkind::text FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r','v','m','p','f') AND n.nspname NOT IN {HIDDEN} \
             ORDER BY n.nspname, c.relname"
        ))?;

        // Everything past the relation list is best-effort: a missing view
        // (Redshift, exotic PG forks, restricted users) must not blank the
        // whole tree.
        let cols = self.simple_rows_or_empty(&format!(
            "SELECT n.nspname, c.relname, a.attname, \
                    format_type(a.atttypid, a.atttypmod), \
                    a.attnotnull::text, pg_get_expr(d.adbin, d.adrelid) \
             FROM pg_attribute a \
             JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE a.attnum > 0 AND NOT a.attisdropped \
               AND c.relkind IN ('r','v','m','p','f') AND n.nspname NOT IN {HIDDEN} \
             ORDER BY n.nspname, c.relname, a.attnum"
        ));

        // array_position is PG 9.5+; Redshift (and very old PG) fall back to
        // attnum order, which matches the PK column order in practice.
        let pks_exact = self.simple_rows(&format!(
            "SELECT n.nspname, c.relname, a.attname \
             FROM pg_index i \
             JOIN pg_class c ON c.oid = i.indrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey) \
             WHERE i.indisprimary AND n.nspname NOT IN {HIDDEN} \
             ORDER BY n.nspname, c.relname, array_position(i.indkey, a.attnum)"
        ));
        let pks = match pks_exact {
            Ok(rows) => rows,
            Err(_) => self.simple_rows_or_empty(&format!(
                "SELECT n.nspname, c.relname, a.attname \
                 FROM pg_index i \
                 JOIN pg_class c ON c.oid = i.indrelid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey) \
                 WHERE i.indisprimary AND n.nspname NOT IN {HIDDEN} \
                 ORDER BY n.nspname, c.relname, a.attnum"
            )),
        };

        // Redshift has no indexes (and no pg_indexes view) — skip the query.
        let indexes = if self.redshift {
            Vec::new()
        } else {
            self.simple_rows_or_empty(&format!(
                "SELECT schemaname, tablename, indexdef FROM pg_indexes \
                 WHERE schemaname NOT IN {HIDDEN} ORDER BY schemaname, tablename, indexname"
            ))
        };

        let fks = self.simple_rows_or_empty(&format!(
            "SELECT n.nspname, c.relname, pg_get_constraintdef(t.oid) \
             FROM pg_constraint t \
             JOIN pg_class c ON c.oid = t.conrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE t.contype = 'f' AND n.nspname NOT IN {HIDDEN}"
        ));

        let mut schemas: BTreeMap<String, BTreeMap<String, TableInfo>> = BTreeMap::new();
        for row in rels {
            let (ns, name, kind) = (unwrap3(&row, 0), unwrap3(&row, 1), unwrap3(&row, 2));
            let kind = match kind.as_str() {
                "v" => "view",
                "m" => "materialized view",
                "f" => "foreign table",
                _ => "table",
            };
            schemas
                .entry(ns.clone())
                .or_default()
                .insert(name.clone(), TableInfo::new(name, kind));
        }
        for row in cols {
            let (ns, tbl) = (unwrap3(&row, 0), unwrap3(&row, 1));
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                t.columns.push(ColumnInfo {
                    name: unwrap3(&row, 2),
                    data_type: unwrap3(&row, 3),
                    nullable: unwrap3(&row, 4) != "true" && unwrap3(&row, 4) != "t",
                    default: row.get(5).and_then(|v| v.clone()),
                });
            }
        }
        for row in pks {
            let (ns, tbl) = (unwrap3(&row, 0), unwrap3(&row, 1));
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                t.pk.push(unwrap3(&row, 2));
            }
        }
        for row in indexes {
            let (ns, tbl) = (unwrap3(&row, 0), unwrap3(&row, 1));
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                t.indexes.push(unwrap3(&row, 2));
            }
        }
        for row in fks {
            let (ns, tbl) = (unwrap3(&row, 0), unwrap3(&row, 1));
            if let Some(t) = schemas.get_mut(&ns).and_then(|s| s.get_mut(&tbl)) {
                t.foreign_keys.push(unwrap3(&row, 2));
            }
        }

        Ok(Catalog {
            schemas: schemas
                .into_iter()
                .map(|(name, tables)| SchemaInfo {
                    name,
                    tables: tables.into_values().collect(),
                })
                .collect(),
        })
    }

    fn native_ddl(&mut self, schema: &str, table: &str) -> AppResult<Option<String>> {
        // Views get their definition straight from the server; tables are
        // generated from catalog metadata by the generic generator.
        let esc_s = schema.replace('\'', "''");
        let esc_t = table.replace('\'', "''");
        let rows = self.simple_rows(&format!(
            "SELECT c.relkind::text, pg_get_viewdef(c.oid, true) \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = '{esc_s}' AND c.relname = '{esc_t}' AND c.relkind IN ('v','m')"
        ))?;
        if let Some(row) = rows.first() {
            let kind = unwrap3(row, 0);
            let def = unwrap3(row, 1);
            let q = Dialect::Postgres;
            let name = format!("{}.{}", q.quote_ident(schema), q.quote_ident(table));
            let head = if kind == "m" {
                format!("CREATE MATERIALIZED VIEW {name} AS")
            } else {
                format!("CREATE OR REPLACE VIEW {name} AS")
            };
            return Ok(Some(format!("{head}\n{def}")));
        }
        Ok(None)
    }
}

fn unwrap3(row: &[Option<String>], i: usize) -> String {
    row.get(i).and_then(|v| v.clone()).unwrap_or_default()
}
