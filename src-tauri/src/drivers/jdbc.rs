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

use jni::objects::{GlobalRef, JObject, JObjectArray, JString, JValue};
use jni::{AttachGuard, InitArgsBuilder, JNIVersion, JavaVM};
use once_cell::sync::OnceCell;

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, DbConnection, Dialect, ExecMeta, JdbcParams, RowEvent, RowSink};

static JVM: OnceCell<Result<JavaVM, String>> = OnceCell::new();

fn jvm() -> AppResult<&'static JavaVM> {
    JVM.get_or_init(|| {
        let args = InitArgsBuilder::new()
            .version(JNIVersion::V8)
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
    let java_on_path = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p).any(|d| {
                d.join("java").exists() || d.join("java.exe").exists()
            })
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

/// Turn a pending Java exception (or plain JNI error) into an AppError.
fn jerr(env: &mut AttachGuard<'_>, e: jni::errors::Error, context: &str) -> AppError {
    if let jni::errors::Error::JavaException = e {
        if let Ok(ex) = env.exception_occurred() {
            let _ = env.exception_clear();
            if !ex.is_null() {
                if let Ok(msg) = env.call_method(&ex, "toString", "()Ljava/lang/String;", &[]) {
                    if let Ok(obj) = msg.l() {
                        if let Ok(s) = env.get_string(&JString::from(obj)) {
                            let s: String = s.into();
                            return AppError::query(s);
                        }
                    }
                }
            }
        }
        let _ = env.exception_clear();
    }
    AppError::query(format!("{context}: {e}"))
}

pub struct JdbcConn {
    conn: GlobalRef,
    _loader: GlobalRef,
}

impl JdbcConn {
    pub fn open(params: &JdbcParams) -> AppResult<Self> {
        let vm = jvm()?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| AppError::driver(e.to_string()))?;

        let result = (|| -> Result<(GlobalRef, GlobalRef), jni::errors::Error> {
            // URL[] urls = { new File(jar).toURI().toURL(), ... }
            let url_class = env.find_class("java/net/URL")?;
            let urls: JObjectArray =
                env.new_object_array(params.jars.len() as i32, &url_class, JObject::null())?;
            for (i, jar) in params.jars.iter().enumerate() {
                let path = env.new_string(jar)?;
                let file = env.new_object(
                    "java/io/File",
                    "(Ljava/lang/String;)V",
                    &[JValue::Object(&path)],
                )?;
                let uri = env.call_method(&file, "toURI", "()Ljava/net/URI;", &[])?.l()?;
                let url = env.call_method(&uri, "toURL", "()Ljava/net/URL;", &[])?.l()?;
                env.set_object_array_element(&urls, i as i32, &url)?;
            }

            // ClassLoader loader = URLClassLoader.newInstance(urls);
            let loader = env
                .call_static_method(
                    "java/net/URLClassLoader",
                    "newInstance",
                    "([Ljava/net/URL;)Ljava/net/URLClassLoader;",
                    &[JValue::Object(&urls)],
                )?
                .l()?;

            // Class<?> cls = Class.forName(driverClass, true, loader);
            let cls_name = env.new_string(&params.driver_class)?;
            let cls = env
                .call_static_method(
                    "java/lang/Class",
                    "forName",
                    "(Ljava/lang/String;ZLjava/lang/ClassLoader;)Ljava/lang/Class;",
                    &[
                        JValue::Object(&cls_name),
                        JValue::Bool(1),
                        JValue::Object(&loader),
                    ],
                )?
                .l()?;

            // Driver driver = (Driver) cls.getDeclaredConstructor().newInstance();
            let empty_classes: JObjectArray =
                env.new_object_array(0, "java/lang/Class", JObject::null())?;
            let ctor = env
                .call_method(
                    &cls,
                    "getDeclaredConstructor",
                    "([Ljava/lang/Class;)Ljava/lang/reflect/Constructor;",
                    &[JValue::Object(&empty_classes)],
                )?
                .l()?;
            let empty_args: JObjectArray =
                env.new_object_array(0, "java/lang/Object", JObject::null())?;
            let driver = env
                .call_method(
                    &ctor,
                    "newInstance",
                    "([Ljava/lang/Object;)Ljava/lang/Object;",
                    &[JValue::Object(&empty_args)],
                )?
                .l()?;

            // Properties props; props.setProperty("user"/"password", ...)
            let props = env.new_object("java/util/Properties", "()V", &[])?;
            for (k, v) in [("user", &params.user), ("password", &params.password)] {
                if !v.is_empty() {
                    let jk = env.new_string(k)?;
                    let jv = env.new_string(v)?;
                    env.call_method(
                        &props,
                        "setProperty",
                        "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/Object;",
                        &[JValue::Object(&jk), JValue::Object(&jv)],
                    )?;
                }
            }

            // Connection conn = driver.connect(url, props);
            let jurl = env.new_string(&params.url)?;
            let conn = env
                .call_method(
                    &driver,
                    "connect",
                    "(Ljava/lang/String;Ljava/util/Properties;)Ljava/sql/Connection;",
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
            Err(e) => Err(AppError::connect(jerr(&mut env, e, "JDBC connect").message)),
        }
    }

    fn with_env<T>(
        &self,
        f: impl FnOnce(&mut AttachGuard<'_>) -> Result<T, jni::errors::Error>,
        context: &str,
    ) -> AppResult<T> {
        let vm = jvm()?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|e| AppError::driver(e.to_string()))?;
        f(&mut env).map_err(|e| jerr(&mut env, e, context))
    }
}

/// Read result-set metadata: (labels, type names).
fn rs_columns(
    env: &mut AttachGuard<'_>,
    rs: &JObject,
) -> Result<Vec<ColumnMeta>, jni::errors::Error> {
    let md = env
        .call_method(rs, "getMetaData", "()Ljava/sql/ResultSetMetaData;", &[])?
        .l()?;
    let n = env.call_method(&md, "getColumnCount", "()I", &[])?.i()?;
    let mut cols = Vec::with_capacity(n as usize);
    for i in 1..=n {
        let label_obj = env
            .call_method(&md, "getColumnLabel", "(I)Ljava/lang/String;", &[JValue::Int(i)])?
            .l()?;
        let label = get_str(env, label_obj)?.unwrap_or_default();
        let ty_obj = env
            .call_method(&md, "getColumnTypeName", "(I)Ljava/lang/String;", &[JValue::Int(i)])?
            .l()?;
        let ty = get_str(env, ty_obj)?.unwrap_or_default();
        cols.push(ColumnMeta::new(label, ty.to_ascii_lowercase()));
    }
    Ok(cols)
}

fn get_str(
    env: &mut AttachGuard<'_>,
    obj: JObject,
) -> Result<Option<String>, jni::errors::Error> {
    if obj.is_null() {
        return Ok(None);
    }
    let js = JString::from(obj);
    let s: String = env.get_string(&js)?.into();
    env.delete_local_ref(js)?;
    Ok(Some(s))
}

/// Iterate a ResultSet, pushing text rows into the sink. Returns stopped_early.
fn drain_rs(
    env: &mut AttachGuard<'_>,
    rs: &JObject,
    ncols: usize,
    cancel: &AtomicBool,
    on_row: &mut RowSink,
) -> Result<(bool, Option<AppError>), jni::errors::Error> {
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok((true, Some(AppError::cancelled())));
        }
        let has = env.call_method(rs, "next", "()Z", &[])?.z()?;
        if !has {
            return Ok((false, None));
        }
        let mut row: Vec<DbValue> = Vec::with_capacity(ncols);
        for i in 1..=ncols as i32 {
            let obj = env
                .call_method(rs, "getString", "(I)Ljava/lang/String;", &[JValue::Int(i)])?
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
                    .call_method(conn, "createStatement", "()Ljava/sql/Statement;", &[])?
                    .l()?;
                let _ = env.call_method(&stmt, "setFetchSize", "(I)V", &[JValue::Int(1000)]);
                let jsql = env.new_string(sql)?;
                let has_rs = env
                    .call_method(&stmt, "execute", "(Ljava/lang/String;)Z", &[JValue::Object(&jsql)])?
                    .z()?;

                let meta = if has_rs {
                    let rs = env
                        .call_method(&stmt, "getResultSet", "()Ljava/sql/ResultSet;", &[])?
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
                    let _ = env.call_method(&rs, "close", "()V", &[]);
                    ExecMeta { columns: Some(columns), affected: None, stopped_early }
                } else {
                    let n = env.call_method(&stmt, "getUpdateCount", "()I", &[])?.i()?;
                    ExecMeta {
                        columns: None,
                        affected: if n >= 0 { Some(n as u64) } else { None },
                        stopped_early: false,
                    }
                };
                let _ = env.call_method(&stmt, "close", "()V", &[]);
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
                    .call_method(conn, "getMetaData", "()Ljava/sql/DatabaseMetaData;", &[])?
                    .l()?;

                // getTables(null, null, "%", {"TABLE","VIEW"})
                let pattern = env.new_string("%")?;
                let types: JObjectArray =
                    env.new_object_array(2, "java/lang/String", JObject::null())?;
                let t0 = env.new_string("TABLE")?;
                let t1 = env.new_string("VIEW")?;
                env.set_object_array_element(&types, 0, &t0)?;
                env.set_object_array_element(&types, 1, &t1)?;
                let rs = env
                    .call_method(
                        &md,
                        "getTables",
                        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;)Ljava/sql/ResultSet;",
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
                let _ = env.call_method(&rs, "close", "()V", &[]);

                // getColumns(null, null, "%", "%")
                let rs = env
                    .call_method(
                        &md,
                        "getColumns",
                        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/sql/ResultSet;",
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
                let _ = env.call_method(&rs, "close", "()V", &[]);
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
                let _ = env.call_method(conn, "close", "()V", &[]);
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
