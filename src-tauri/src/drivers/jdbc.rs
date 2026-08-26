//! JDBC bridge via JNI.
//!
//! The JVM is discovered and loaded at runtime (`jni` crate `invocation`
//! feature uses java-locator + dynamic loading), so the app runs fine on
//! machines without Java — JDBC connections just report a friendly error.
//! Driver jars are loaded through a per-connection `URLClassLoader` and the
//! driver is instantiated directly (bypassing `DriverManager`, which ignores
//! drivers from child classloaders).

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use jni::objects::{JObject, JObjectArray, JString, JValue};
use jni::refs::Global;
use jni::{jni_sig, jni_str, Env, InitArgsBuilder, JNIVersion, JavaVM};
use once_cell::sync::OnceCell;

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, DbConnection, Dialect, ExecMeta, JdbcParams, RowEvent, RowSink};

static JVM: OnceCell<Result<JavaVM, String>> = OnceCell::new();

fn jvm() -> AppResult<&'static JavaVM> {
    JVM.get_or_init(|| {
        let args = InitArgsBuilder::new()
            .version(JNIVersion::V1_8)
            .option("-Djava.awt.headless=true")
            .option("-Xrs")
            .build()
            .map_err(|e| e.to_string())?;
        JavaVM::new(args).map_err(|e| {
            format!(
                "Could not start a Java VM ({e}). JDBC connections need a Java 11+ \
                 runtime — install one (e.g. Temurin from adoptium.net) and make sure \
                 JAVA_HOME is set."
            )
        })
    })
    .as_ref()
    .map_err(|e| AppError::driver(e.clone()))
}

/// Availability status for the UI. Cheap: reports on JAVA_HOME/PATH without
/// booting a VM (unless one is already running).
pub fn status() -> (bool, String) {
    if let Some(Ok(_)) = JVM.get() {
        return (true, "Java VM running".into());
    }
    if let Ok(home) = std::env::var("JAVA_HOME") {
        if !home.is_empty() {
            return (true, format!("JAVA_HOME = {home}"));
        }
    }
    // On macOS /usr/bin/java is a stub that exists even with no JVM
    // installed; ask java_home instead of trusting PATH.
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("/usr/libexec/java_home").output() {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    return (true, format!("JVM at {p}"));
                }
            }
        }
        (
            false,
            "No Java runtime found. JDBC connections need a Java 11+ runtime \
             (e.g. Temurin from adoptium.net) — install one or set JAVA_HOME."
                .into(),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let java_on_path = std::env::var_os("PATH")
            .map(|p| {
                std::env::split_paths(&p)
                    .any(|d| d.join("java").exists() || d.join("java.exe").exists())
            })
            .unwrap_or(false);
        if java_on_path {
            (true, "java found on PATH".into())
        } else {
            (
                false,
                "No Java runtime found. JDBC connections need a Java 11+ runtime \
                 (e.g. Temurin from adoptium.net)."
                    .into(),
            )
        }
    }
}

/// Turn a pending Java exception (or plain JNI error) into an AppError.
/// The thread-attachment APIs catch pending exceptions themselves and report
/// them as `CaughtJavaException`, carrying the throwable with them.
fn jerr(env: &mut Env<'_>, e: jni::errors::Error, context: &str) -> AppError {
    match &e {
        // Already caught and cleared for us, with the Java-side class and
        // message decoded — same text `Throwable.toString()` would produce.
        jni::errors::Error::CaughtJavaException { name, msg, .. } => {
            AppError::query(if msg.is_empty() {
                name.clone()
            } else {
                format!("{name}: {msg}")
            })
        }
        // Still pending: raised inside a frame we own, so read it ourselves.
        jni::errors::Error::JavaException => {
            let pending = env.exception_occurred();
            env.exception_clear();
            if let Some(ex) = pending {
                if let Ok(obj) = env
                    .call_method(&ex, jni_str!("toString"), jni_sig!("()Ljava/lang/String;"), &[])
                    .and_then(|v| v.l())
                {
                    if let Ok(js) = env.cast_local::<JString>(obj) {
                        if let Ok(s) = js.try_to_string(env) {
                            return AppError::query(s);
                        }
                    }
                }
            }
            AppError::query(format!("{context}: {e}"))
        }
        _ => AppError::query(format!("{context}: {e}")),
    }
}

pub struct JdbcConn {
    conn: Global<JObject<'static>>,
    _loader: Global<JObject<'static>>,
}

impl JdbcConn {
    pub fn open(params: &JdbcParams) -> AppResult<Self> {
        let vm = jvm()?;
        vm.attach_current_thread(|env| -> jni::errors::Result<AppResult<Self>> {
            Ok(Self::open_attached(params, env))
        })
        .map_err(|e| AppError::driver(e.to_string()))?
    }

    fn open_attached(params: &JdbcParams, env: &mut Env<'_>) -> AppResult<Self> {
        type Refs = (Global<JObject<'static>>, Global<JObject<'static>>);
        let result = (|| -> Result<Refs, jni::errors::Error> {
            // URL[] urls = { new File(jar).toURI().toURL(), ... }
            let url_class = env.find_class(jni_str!("java/net/URL"))?;
            let urls: JObjectArray<JObject> =
                env.new_object_array(params.jars.len() as i32, &url_class, JObject::null())?;
            for (i, jar) in params.jars.iter().enumerate() {
                let path = env.new_string(jar)?;
                let file = env.new_object(
                    jni_str!("java/io/File"),
                    jni_sig!("(Ljava/lang/String;)V"),
                    &[JValue::Object(&path)],
                )?;
                let uri = env.call_method(
                    &file,
                    jni_str!("toURI"),
                    jni_sig!("()Ljava/net/URI;"),
                    &[],
                )?.l()?;
                let url = env.call_method(
                    &uri,
                    jni_str!("toURL"),
                    jni_sig!("()Ljava/net/URL;"),
                    &[],
                )?.l()?;
                urls.set_element(env, i, &url)?;
            }

            // ClassLoader loader = URLClassLoader.newInstance(urls);
            let loader = env
                .call_static_method(
                    jni_str!("java/net/URLClassLoader"),
                    jni_str!("newInstance"),
                    jni_sig!("([Ljava/net/URL;)Ljava/net/URLClassLoader;"),
                    &[JValue::Object(&urls)],
                )?
                .l()?;

            // Class<?> cls = Class.forName(driverClass, true, loader);
            let cls_name = env.new_string(&params.driver_class)?;
            let cls = env
                .call_static_method(
                    jni_str!("java/lang/Class"),
                    jni_str!("forName"),
                    jni_sig!("(Ljava/lang/String;ZLjava/lang/ClassLoader;)Ljava/lang/Class;"),
                    &[
                        JValue::Object(&cls_name),
                        JValue::Bool(true),
                        JValue::Object(&loader),
                    ],
                )?
                .l()?;

            // Driver driver = (Driver) cls.getDeclaredConstructor().newInstance();
            let empty_classes: JObjectArray =
                env.new_object_array(0, jni_str!("java/lang/Class"), JObject::null())?;
            let ctor = env
                .call_method(
                    &cls,
                    jni_str!("getDeclaredConstructor"),
                    jni_sig!("([Ljava/lang/Class;)Ljava/lang/reflect/Constructor;"),
                    &[JValue::Object(&empty_classes)],
                )?
                .l()?;
            let empty_args: JObjectArray =
                env.new_object_array(0, jni_str!("java/lang/Object"), JObject::null())?;
            let driver = env
                .call_method(
                    &ctor,
                    jni_str!("newInstance"),
                    jni_sig!("([Ljava/lang/Object;)Ljava/lang/Object;"),
                    &[JValue::Object(&empty_args)],
                )?
                .l()?;

            // Properties props; props.setProperty("user"/"password", ...)
            let props = env.new_object(jni_str!("java/util/Properties"), jni_sig!("()V"), &[])?;
            for (k, v) in [("user", &params.user), ("password", &params.password)] {
                if !v.is_empty() {
                    let jk = env.new_string(k)?;
                    let jv = env.new_string(v)?;
                    env.call_method(
                        &props,
                        jni_str!("setProperty"),
                        jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/Object;"),
                        &[JValue::Object(&jk), JValue::Object(&jv)],
                    )?;
                }
            }

            // Connection conn = driver.connect(url, props);
            let jurl = env.new_string(&params.url)?;
            let conn = env
                .call_method(
                    &driver,
                    jni_str!("connect"),
                    jni_sig!("(Ljava/lang/String;Ljava/util/Properties;)Ljava/sql/Connection;"),
                    &[JValue::Object(&jurl), JValue::Object(&props)],
                )?
                .l()?;

            Ok((env.new_global_ref(&conn)?, env.new_global_ref(&loader)?))
        })();

        match result {
            Ok((conn, loader)) => {
                if conn.as_obj().is_null() {
                    return Err(AppError::connect(format!(
                        "{} did not accept the URL '{}'",
                        params.driver_class, params.url
                    )));
                }
                Ok(Self { conn, _loader: loader })
            }
            Err(e) => Err(AppError::connect(jerr(env, e, "JDBC connect").message)),
        }
    }

    fn with_env<T>(
        &self,
        f: impl FnOnce(&mut Env<'_>) -> Result<T, jni::errors::Error>,
        context: &str,
    ) -> AppResult<T> {
        let vm = jvm()?;
        // The attachment pushes its own JNI stack frame, so locals created in
        // `f` are released when it returns.
        vm.attach_current_thread(|env| -> jni::errors::Result<AppResult<T>> {
            Ok(f(env).map_err(|e| jerr(env, e, context)))
        })
            .map_err(|e| AppError::driver(e.to_string()))?
    }
}

/// Read result-set metadata: (labels, type names).
fn rs_columns(
    env: &mut Env<'_>,
    rs: &JObject,
) -> Result<Vec<ColumnMeta>, jni::errors::Error> {
    let md = env
        .call_method(rs, jni_str!("getMetaData"), jni_sig!("()Ljava/sql/ResultSetMetaData;"), &[])?
        .l()?;
    let n = env.call_method(&md, jni_str!("getColumnCount"), jni_sig!("()I"), &[])?.i()?;
    let mut cols = Vec::with_capacity(n as usize);
    for i in 1..=n {
        let label_obj = env
            .call_method(
                &md,
                jni_str!("getColumnLabel"),
                jni_sig!("(I)Ljava/lang/String;"),
                &[JValue::Int(i)],
            )?
            .l()?;
        let label = get_str(env, label_obj)?.unwrap_or_default();
        let ty_obj = env
            .call_method(
                &md,
                jni_str!("getColumnTypeName"),
                jni_sig!("(I)Ljava/lang/String;"),
                &[JValue::Int(i)],
            )?
            .l()?;
        let ty = get_str(env, ty_obj)?.unwrap_or_default();
        cols.push(ColumnMeta::new(label, ty.to_ascii_lowercase()));
    }
    Ok(cols)
}

fn get_str(
    env: &mut Env<'_>,
    obj: JObject,
) -> Result<Option<String>, jni::errors::Error> {
    if obj.is_null() {
        return Ok(None);
    }
    let js = env.cast_local::<JString>(obj)?;
    let s: String = js.try_to_string(env)?;
    env.delete_local_ref(js);
    Ok(Some(s))
}

/// Iterate a ResultSet, pushing text rows into the sink. Returns stopped_early.
fn drain_rs(
    env: &mut Env<'_>,
    rs: &JObject,
    ncols: usize,
    cancel: &AtomicBool,
    on_row: &mut RowSink,
) -> Result<(bool, Option<AppError>), jni::errors::Error> {
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok((true, Some(AppError::cancelled())));
        }
        let has = env.call_method(rs, jni_str!("next"), jni_sig!("()Z"), &[])?.z()?;
        if !has {
            return Ok((false, None));
        }
        let mut row: Vec<DbValue> = Vec::with_capacity(ncols);
        for i in 1..=ncols as i32 {
            let obj = env
                .call_method(
                    rs,
                    jni_str!("getString"),
                    jni_sig!("(I)Ljava/lang/String;"),
                    &[JValue::Int(i)],
                )?
                .l()?;
            let s = get_str(env, obj)?;
            row.push(match s {
                None => DbValue::Null,
                Some(t) => DbValue::Text(t),
            });
        }
        if let ControlFlow::Break(()) = on_row(RowEvent::Row(row)) {
            return Ok((true, None));
        }
    }
}

impl DbConnection for JdbcConn {
    fn dialect(&self) -> Dialect {
        Dialect::Generic
    }

    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_row: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        check_cancel(cancel)?;
        let conn = self.conn.as_obj();
        let mut inner_err: Option<AppError> = None;
        let meta = self.with_env(
            |env| {
                let stmt = env
                    .call_method(
                        conn,
                        jni_str!("createStatement"),
                        jni_sig!("()Ljava/sql/Statement;"),
                        &[],
                    )?
                    .l()?;
                let _ = env.call_method(
                    &stmt,
                    jni_str!("setFetchSize"),
                    jni_sig!("(I)V"),
                    &[JValue::Int(1000)],
                );
                let jsql = env.new_string(sql)?;
                let has_rs = env
                    .call_method(
                        &stmt,
                        jni_str!("execute"),
                        jni_sig!("(Ljava/lang/String;)Z"),
                        &[JValue::Object(&jsql)],
                    )?
                    .z()?;

                let meta = if has_rs {
                    let rs = env
                        .call_method(
                            &stmt,
                            jni_str!("getResultSet"),
                            jni_sig!("()Ljava/sql/ResultSet;"),
                            &[],
                        )?
                        .l()?;
                    let columns = rs_columns(env, &rs)?;
                    let mut stopped_by_columns = false;
                    if let ControlFlow::Break(()) = on_row(RowEvent::Columns(columns.clone())) {
                        stopped_by_columns = true;
                    }
                    let (stopped_early, err) = if stopped_by_columns {
                        (true, None)
                    } else {
                        drain_rs(env, &rs, columns.len(), cancel, on_row)?
                    };
                    inner_err = err;
                    let _ = env.call_method(&rs, jni_str!("close"), jni_sig!("()V"), &[]);
                    ExecMeta { columns: Some(columns), affected: None, stopped_early }
                } else {
                    let n = env.call_method(
                        &stmt,
                        jni_str!("getUpdateCount"),
                        jni_sig!("()I"),
                        &[],
                    )?.i()?;
                    ExecMeta {
                        columns: None,
                        affected: if n >= 0 { Some(n as u64) } else { None },
                        stopped_early: false,
                    }
                };
                let _ = env.call_method(&stmt, jni_str!("close"), jni_sig!("()V"), &[]);
                Ok(meta)
            },
            "JDBC execute",
        )?;
        if let Some(e) = inner_err {
            return Err(e);
        }
        Ok(meta)
    }

    fn catalog(&mut self) -> AppResult<Catalog> {
        let none = AtomicBool::new(false);
        let conn = self.conn.as_obj();

        let mut tables_rows: Vec<Vec<DbValue>> = Vec::new();
        let mut col_rows: Vec<Vec<DbValue>> = Vec::new();

        self.with_env(
            |env| {
                let md = env
                    .call_method(
                        conn,
                        jni_str!("getMetaData"),
                        jni_sig!("()Ljava/sql/DatabaseMetaData;"),
                        &[],
                    )?
                    .l()?;

                // getTables(null, null, "%", {"TABLE","VIEW"})
                let pattern = env.new_string("%")?;
                let types: JObjectArray =
                    env.new_object_array(2, jni_str!("java/lang/String"), JObject::null())?;
                let t0 = env.new_string("TABLE")?;
                let t1 = env.new_string("VIEW")?;
                types.set_element(env, 0, &t0)?;
                types.set_element(env, 1, &t1)?;
                let rs = env
                    .call_method(
                        &md,
                        jni_str!("getTables"),
                        jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;)Ljava/sql/ResultSet;"),
                        &[
                            JValue::Object(&JObject::null()),
                            JValue::Object(&JObject::null()),
                            JValue::Object(&pattern),
                            JValue::Object(&types),
                        ],
                    )?
                    .l()?;
                let mut sink = |ev: RowEvent| {
                    if let RowEvent::Row(row) = ev {
                        tables_rows.push(row);
                    }
                    ControlFlow::Continue(())
                };
                drain_rs(env, &rs, 4, &none, &mut sink)?;
                let _ = env.call_method(&rs, jni_str!("close"), jni_sig!("()V"), &[]);

                // getColumns(null, null, "%", "%")
                let rs = env
                    .call_method(
                        &md,
                        jni_str!("getColumns"),
                        jni_sig!("(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/sql/ResultSet;"),
                        &[
                            JValue::Object(&JObject::null()),
                            JValue::Object(&JObject::null()),
                            JValue::Object(&pattern),
                            JValue::Object(&pattern),
                        ],
                    )?
                    .l()?;
                let mut sink = |ev: RowEvent| {
                    if let RowEvent::Row(row) = ev {
                        col_rows.push(row);
                    }
                    ControlFlow::Continue(())
                };
                // Need columns 1..18; drain first 18.
                drain_rs(env, &rs, 18, &none, &mut sink)?;
                let _ = env.call_method(&rs, jni_str!("close"), jni_sig!("()V"), &[]);
                Ok(())
            },
            "JDBC catalog",
        )?;

        let mut schemas: BTreeMap<String, BTreeMap<String, TableInfo>> = BTreeMap::new();
        for row in &tables_rows {
            // 1 TABLE_CAT, 2 TABLE_SCHEM, 3 TABLE_NAME, 4 TABLE_TYPE (1-based)
            let key = super::odbc_schema_key(&txt(row, 0), &txt(row, 1));
            let name = txt(row, 2);
            if name.is_empty() {
                continue;
            }
            let kind = if txt(row, 3).to_ascii_uppercase().contains("VIEW") { "view" } else { "table" };
            schemas
                .entry(key)
                .or_default()
                .insert(name.clone(), TableInfo::new(name, kind));
        }
        for row in &col_rows {
            // 4 COLUMN_NAME, 6 TYPE_NAME, 7 COLUMN_SIZE, 13 COLUMN_DEF, 18 IS_NULLABLE
            let key = super::odbc_schema_key(&txt(row, 0), &txt(row, 1));
            let tbl = txt(row, 2);
            if let Some(t) = schemas.get_mut(&key).and_then(|s| s.get_mut(&tbl)) {
                t.columns.push(ColumnInfo {
                    name: txt(row, 3),
                    data_type: txt(row, 5).to_ascii_lowercase(),
                    nullable: txt(row, 17) != "NO",
                    default: {
                        let d = txt(row, 12);
                        if d.is_empty() { None } else { Some(d) }
                    },
                });
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
}

impl Drop for JdbcConn {
    fn drop(&mut self) {
        let conn = self.conn.as_obj();
        let _ = self.with_env(
            |env| {
                let _ = env.call_method(conn, jni_str!("close"), jni_sig!("()V"), &[]);
                Ok(())
            },
            "JDBC close",
        );
    }
}

fn txt(row: &[DbValue], i: usize) -> String {
    match row.get(i) {
        Some(DbValue::Text(s)) => s.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end JDBC round-trip against H2 in-memory. Opt-in: needs a JVM
    /// and an H2 jar, so it is ignored by default.
    ///   QUERYDECK_TEST_H2_JAR=/path/to/h2.jar \
    ///     cargo test --lib drivers::jdbc -- --ignored --nocapture
    #[test]
    #[ignore]
    fn h2_roundtrip() {
        let Ok(jar) = std::env::var("QUERYDECK_TEST_H2_JAR") else {
            panic!("set QUERYDECK_TEST_H2_JAR to an h2 jar path");
        };
        let params = JdbcParams {
            url: "jdbc:h2:mem:qdtest;DB_CLOSE_DELAY=-1".into(),
            driver_class: "org.h2.Driver".into(),
            jars: vec![jar],
            user: "sa".into(),
            password: String::new(),
        };
        let mut conn = JdbcConn::open(&params).expect("open");

        conn.ping().expect("ping");

        let cancel = AtomicBool::new(false);
        let mut sink = |_: RowEvent| ControlFlow::Continue(());
        conn.execute(
            "CREATE TABLE items (id INT PRIMARY KEY, name VARCHAR(32))",
            &cancel,
            &mut sink,
        )
        .expect("create");
        conn.execute(
            "INSERT INTO items VALUES (1, 'alpha'), (2, 'beta')",
            &cancel,
            &mut sink,
        )
        .expect("insert");

        let mut cols: Vec<ColumnMeta> = Vec::new();
        let mut rows: Vec<Vec<DbValue>> = Vec::new();
        let mut collect = |ev: RowEvent| -> ControlFlow<()> {
            match ev {
                RowEvent::Columns(c) => cols = c,
                RowEvent::Row(r) => rows.push(r),
            }
            ControlFlow::Continue(())
        };
        let meta = conn
            .execute("SELECT id, name FROM items ORDER BY id", &cancel, &mut collect)
            .expect("select");

        assert_eq!(cols.len(), 2, "column count");
        assert_eq!(rows.len(), 2, "row count");
        assert_eq!(txt(&rows[0], 1), "alpha");
        assert_eq!(txt(&rows[1], 1), "beta");
        assert!(meta.columns.is_some());

        let catalog = conn.catalog().expect("catalog");
        let found = catalog
            .schemas
            .iter()
            .flat_map(|s| s.tables.iter())
            .any(|t| t.name.eq_ignore_ascii_case("items"));
        assert!(found, "catalog should list the items table");

        // Java-side failures must surface as readable messages, not bare JNI
        // error codes — the exception-mapping path has its own quirks.
        let err = conn
            .execute("SELECT * FROM no_such_table", &cancel, &mut sink)
            .expect_err("bad SQL should fail");
        assert!(
            err.message.to_lowercase().contains("no_such_table"),
            "exception text should name the missing table, got: {}",
            err.message
        );

        println!(
            "jdbc round-trip OK: {} cols, {} rows\nerror text: {}",
            cols.len(),
            rows.len(),
            err.message.lines().next().unwrap_or_default()
        );
    }
}
