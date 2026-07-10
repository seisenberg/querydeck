//! Minimal ODBC binding, loaded dynamically at runtime.
//!
//! We bind the driver manager with `libloading` instead of linking, so the
//! app runs on machines with no ODBC installed (the ODBC connection type just
//! reports a friendly error). On Windows we use the wide (UTF-16) entry
//! points of the built-in `odbc32.dll`; on macOS/Linux we use the ANSI entry
//! points of unixODBC with UTF-8, which modern drivers handle.

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::ops::ControlFlow;
use std::ptr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use libloading::Library;
use once_cell::sync::OnceCell;

use crate::catalog::{Catalog, ColumnInfo, SchemaInfo, TableInfo};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

use super::{check_cancel, odbc_schema_key as schema_key, DbConnection, Dialect, ExecMeta, OdbcParams, RowEvent, RowSink};

type Handle = *mut c_void;
type SqlReturn = i16;

#[cfg(windows)]
type SqlChar = u16;
#[cfg(not(windows))]
type SqlChar = u8;

const SQL_HANDLE_ENV: i16 = 1;
const SQL_HANDLE_DBC: i16 = 2;
const SQL_HANDLE_STMT: i16 = 3;
const SQL_SUCCESS: i16 = 0;
const SQL_SUCCESS_WITH_INFO: i16 = 1;
const SQL_NO_DATA: i16 = 100;
const SQL_NULL_DATA: isize = -1;
const SQL_NO_TOTAL: isize = -4;
const SQL_NTS: i32 = -3;
const SQL_ATTR_ODBC_VERSION: i32 = 200;
const SQL_OV_ODBC3: usize = 3;
const SQL_DRIVER_NOPROMPT: u16 = 0;
#[cfg(windows)]
const SQL_C_TEXT: i16 = -8; // SQL_C_WCHAR
#[cfg(not(windows))]
const SQL_C_TEXT: i16 = 1; // SQL_C_CHAR

fn ok(r: SqlReturn) -> bool {
    r == SQL_SUCCESS || r == SQL_SUCCESS_WITH_INFO
}

macro_rules! sym {
    ($lib:expr, $name:literal) => {{
        #[cfg(windows)]
        let name = concat!($name, "W\0");
        #[cfg(not(windows))]
        let name = concat!($name, "\0");
        unsafe { $lib.get(name.as_bytes()) }
            .map(|s| *s)
            .map_err(|e| AppError::driver(format!("ODBC symbol {}: {e}", $name)))
    }};
    ($lib:expr, $name:literal, exact) => {{
        unsafe { $lib.get(concat!($name, "\0").as_bytes()) }
            .map(|s| *s)
            .map_err(|e| AppError::driver(format!("ODBC symbol {}: {e}", $name)))
    }};
}

#[allow(non_snake_case, clippy::type_complexity)]
pub struct OdbcLib {
    _lib: Library,
    env: Handle,
    AllocHandle: unsafe extern "system" fn(i16, Handle, *mut Handle) -> SqlReturn,
    FreeHandle: unsafe extern "system" fn(i16, Handle) -> SqlReturn,
    DriverConnect: unsafe extern "system" fn(Handle, Handle, *const SqlChar, i16, *mut SqlChar, i16, *mut i16, u16) -> SqlReturn,
    Disconnect: unsafe extern "system" fn(Handle) -> SqlReturn,
    ExecDirect: unsafe extern "system" fn(Handle, *const SqlChar, i32) -> SqlReturn,
    NumResultCols: unsafe extern "system" fn(Handle, *mut i16) -> SqlReturn,
    DescribeCol: unsafe extern "system" fn(Handle, u16, *mut SqlChar, i16, *mut i16, *mut i16, *mut usize, *mut i16, *mut i16) -> SqlReturn,
    Fetch: unsafe extern "system" fn(Handle) -> SqlReturn,
    GetData: unsafe extern "system" fn(Handle, u16, i16, *mut c_void, isize, *mut isize) -> SqlReturn,
    RowCount: unsafe extern "system" fn(Handle, *mut isize) -> SqlReturn,
    GetDiagRec: unsafe extern "system" fn(i16, Handle, i16, *mut SqlChar, *mut i32, *mut SqlChar, i16, *mut i16) -> SqlReturn,
    Tables: unsafe extern "system" fn(Handle, *const SqlChar, i16, *const SqlChar, i16, *const SqlChar, i16, *const SqlChar, i16) -> SqlReturn,
    Columns: unsafe extern "system" fn(Handle, *const SqlChar, i16, *const SqlChar, i16, *const SqlChar, i16, *const SqlChar, i16) -> SqlReturn,
}

unsafe impl Send for OdbcLib {}
unsafe impl Sync for OdbcLib {}

static ODBC: OnceCell<Result<Arc<OdbcLib>, String>> = OnceCell::new();

fn candidates() -> Vec<String> {
    #[cfg(windows)]
    {
        vec!["odbc32.dll".into()]
    }
    #[cfg(target_os = "macos")]
    {
        let mut c: Vec<String> = vec![
            "libodbc.2.dylib".into(),
            "libodbc.dylib".into(),
            "/opt/homebrew/lib/libodbc.2.dylib".into(),
            "/usr/local/lib/libodbc.2.dylib".into(),
            "/opt/local/lib/libodbc.2.dylib".into(),
        ];
        // User-local Homebrew (no-sudo install).
        if let Some(home) = dirs::home_dir() {
            c.push(home.join("homebrew/lib/libodbc.2.dylib").to_string_lossy().into_owned());
            c.push(home.join(".local/lib/libodbc.2.dylib").to_string_lossy().into_owned());
        }
        c
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut c: Vec<String> =
            vec!["libodbc.so.2".into(), "libodbc.so".into(), "libodbc.so.1".into()];
        if let Some(home) = dirs::home_dir() {
            c.push(home.join(".local/lib/libodbc.so.2").to_string_lossy().into_owned());
        }
        c
    }
}

pub fn load() -> Result<Arc<OdbcLib>, String> {
    ODBC.get_or_init(|| {
        let mut last_err = String::new();
        for name in candidates() {
            match unsafe { Library::new(name) } {
                Ok(lib) => return OdbcLib::init(lib).map(Arc::new).map_err(|e| e.message),
                Err(e) => last_err = e.to_string(),
            }
        }
        Err(format!(
            "ODBC driver manager not found ({last_err}). Install unixODBC \
             (macOS: `brew install unixodbc`, Debian/Ubuntu: `apt install unixodbc`); \
             on Windows ODBC is built in."
        ))
    })
    .clone()
}

/// Human-readable availability status for the UI.
pub fn status() -> (bool, String) {
    match load() {
        Ok(_) => (true, "ODBC driver manager loaded".into()),
        Err(e) => (false, e),
    }
}

impl OdbcLib {
    fn init(lib: Library) -> AppResult<Self> {
        let mut me = Self {
            AllocHandle: sym!(lib, "SQLAllocHandle", exact)?,
            FreeHandle: sym!(lib, "SQLFreeHandle", exact)?,
            DriverConnect: sym!(lib, "SQLDriverConnect")?,
            Disconnect: sym!(lib, "SQLDisconnect", exact)?,
            ExecDirect: sym!(lib, "SQLExecDirect")?,
            NumResultCols: sym!(lib, "SQLNumResultCols", exact)?,
            DescribeCol: sym!(lib, "SQLDescribeCol")?,
            Fetch: sym!(lib, "SQLFetch", exact)?,
            GetData: sym!(lib, "SQLGetData", exact)?,
            RowCount: sym!(lib, "SQLRowCount", exact)?,
            GetDiagRec: sym!(lib, "SQLGetDiagRec")?,
            Tables: sym!(lib, "SQLTables")?,
            Columns: sym!(lib, "SQLColumns")?,
            env: ptr::null_mut(),
            _lib: lib,
        };
        let set_env: unsafe extern "system" fn(Handle, i32, *mut c_void, i32) -> SqlReturn =
            sym!(me._lib, "SQLSetEnvAttr", exact)?;
        unsafe {
            let mut env: Handle = ptr::null_mut();
            if !ok((me.AllocHandle)(SQL_HANDLE_ENV, ptr::null_mut(), &mut env)) {
                return Err(AppError::driver("SQLAllocHandle(ENV) failed"));
            }
            (set_env)(env, SQL_ATTR_ODBC_VERSION, SQL_OV_ODBC3 as *mut c_void, 0);
            me.env = env;
        }
        Ok(me)
    }

    fn diag(&self, handle_type: i16, handle: Handle, context: &str) -> AppError {
        let mut msgs = Vec::new();
        let mut rec = 1i16;
        loop {
            let mut state = [0 as SqlChar; 6];
            let mut native = 0i32;
            let mut buf = vec![0 as SqlChar; 2048];
            let mut len = 0i16;
            let r = unsafe {
                (self.GetDiagRec)(
                    handle_type,
                    handle,
                    rec,
                    state.as_mut_ptr(),
                    &mut native,
                    buf.as_mut_ptr(),
                    buf.len() as i16,
                    &mut len,
                )
            };
            if !ok(r) {
                break;
            }
            let msg = decode(&buf[..(len.max(0) as usize).min(buf.len())]);
            let st = decode(&state[..5]);
            msgs.push(format!("[{st}] {msg}"));
            rec += 1;
            if rec > 8 {
                break;
            }
        }
        if msgs.is_empty() {
            AppError::query(format!("{context}: unknown ODBC error"))
        } else {
            AppError::query(msgs.join("\n"))
        }
    }
}

fn encode(s: &str) -> Vec<SqlChar> {
    #[cfg(windows)]
    {
        s.encode_utf16().collect()
    }
    #[cfg(not(windows))]
    {
        s.as_bytes().to_vec()
    }
}

fn decode(buf: &[SqlChar]) -> String {
    #[cfg(windows)]
    {
        String::from_utf16_lossy(buf)
    }
    #[cfg(not(windows))]
    {
        String::from_utf8_lossy(buf).into_owned()
    }
}

pub struct OdbcConn {
    lib: Arc<OdbcLib>,
    dbc: Handle,
}

unsafe impl Send for OdbcConn {}

impl OdbcConn {
    pub fn open(params: &OdbcParams) -> AppResult<Self> {
        let lib = load().map_err(AppError::driver)?;
        let mut dbc: Handle = ptr::null_mut();
        unsafe {
            if !ok((lib.AllocHandle)(SQL_HANDLE_DBC, lib.env, &mut dbc)) {
                return Err(lib.diag(SQL_HANDLE_ENV, lib.env, "SQLAllocHandle(DBC)"));
            }
            let conn_str = encode(&params.connection_string);
            let mut out = vec![0 as SqlChar; 1024];
            let mut out_len = 0i16;
            let r = (lib.DriverConnect)(
                dbc,
                ptr::null_mut(),
                conn_str.as_ptr(),
                conn_str.len() as i16,
                out.as_mut_ptr(),
                out.len() as i16,
                &mut out_len,
                SQL_DRIVER_NOPROMPT,
            );
            if !ok(r) {
                let err = lib.diag(SQL_HANDLE_DBC, dbc, "connect");
                (lib.FreeHandle)(SQL_HANDLE_DBC, dbc);
                return Err(AppError::connect(err.message));
            }
        }
        Ok(Self { lib, dbc })
    }

    fn alloc_stmt(&self) -> AppResult<Stmt<'_>> {
        let mut stmt: Handle = ptr::null_mut();
        unsafe {
            if !ok((self.lib.AllocHandle)(SQL_HANDLE_STMT, self.dbc, &mut stmt)) {
                return Err(self.lib.diag(SQL_HANDLE_DBC, self.dbc, "SQLAllocHandle(STMT)"));
            }
        }
        Ok(Stmt { conn: self, handle: stmt })
    }

    /// Read all rows of the statement's result set as text.
    fn read_result(
        &self,
        stmt: &Stmt<'_>,
        cancel: &AtomicBool,
        on_row: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        let lib = &self.lib;
        let mut ncols = 0i16;
        unsafe {
            if !ok((lib.NumResultCols)(stmt.handle, &mut ncols)) {
                return Err(lib.diag(SQL_HANDLE_STMT, stmt.handle, "SQLNumResultCols"));
            }
        }

        if ncols <= 0 {
            let mut count: isize = -1;
            unsafe {
                (lib.RowCount)(stmt.handle, &mut count);
            }
            return Ok(ExecMeta {
                columns: None,
                affected: if count >= 0 { Some(count as u64) } else { None },
                stopped_early: false,
            });
        }

        let mut columns = Vec::with_capacity(ncols as usize);
        for i in 1..=ncols as u16 {
            let mut name = vec![0 as SqlChar; 256];
            let (mut name_len, mut data_type, mut decimals, mut nullable) = (0i16, 0i16, 0i16, 0i16);
            let mut size = 0usize;
            unsafe {
                (lib.DescribeCol)(
                    stmt.handle,
                    i,
                    name.as_mut_ptr(),
                    name.len() as i16,
                    &mut name_len,
                    &mut data_type,
                    &mut size,
                    &mut decimals,
                    &mut nullable,
                );
            }
            let cname = decode(&name[..(name_len.max(0) as usize).min(name.len())]);
            columns.push(ColumnMeta::new(cname, sql_type_name(data_type)));
        }

        let mut stopped_early = false;
        if let ControlFlow::Break(()) = on_row(RowEvent::Columns(columns.clone())) {
            return Ok(ExecMeta { columns: Some(columns), affected: None, stopped_early: true });
        }
        loop {
            check_cancel(cancel)?;
            let r = unsafe { (lib.Fetch)(stmt.handle) };
            if r == SQL_NO_DATA {
                break;
            }
            if !ok(r) {
                return Err(lib.diag(SQL_HANDLE_STMT, stmt.handle, "SQLFetch"));
            }
            let mut row = Vec::with_capacity(ncols as usize);
            for i in 1..=ncols as u16 {
                row.push(self.get_text(stmt.handle, i)?);
            }
            if let ControlFlow::Break(()) = on_row(RowEvent::Row(row)) {
                stopped_early = true;
                break;
            }
        }

        Ok(ExecMeta { columns: Some(columns), affected: None, stopped_early })
    }

    fn get_text(&self, stmt: Handle, col: u16) -> AppResult<DbValue> {
        let lib = &self.lib;
        let char_size = std::mem::size_of::<SqlChar>();
        let mut buf = vec![0 as SqlChar; 16384];
        let mut collected: Vec<SqlChar> = Vec::new();
        loop {
            let mut ind: isize = 0;
            let r = unsafe {
                (lib.GetData)(
                    stmt,
                    col,
                    SQL_C_TEXT,
                    buf.as_mut_ptr() as *mut c_void,
                    (buf.len() * char_size) as isize,
                    &mut ind,
                )
            };
            if r == SQL_NO_DATA {
                break;
            }
            if !ok(r) {
                return Err(lib.diag(SQL_HANDLE_STMT, stmt, "SQLGetData"));
            }
            if ind == SQL_NULL_DATA {
                return Ok(DbValue::Null);
            }
            let chunk_chars = if r == SQL_SUCCESS_WITH_INFO && (ind == SQL_NO_TOTAL || ind as usize >= buf.len() * char_size)
            {
                buf.len() - 1 // buffer filled, minus NUL terminator
            } else {
                (ind as usize / char_size).min(buf.len().saturating_sub(1))
            };
            collected.extend_from_slice(&buf[..chunk_chars]);
            if r == SQL_SUCCESS {
                break;
            }
        }
        Ok(DbValue::Text(decode(&collected)))
    }
}

impl Drop for OdbcConn {
    fn drop(&mut self) {
        unsafe {
            (self.lib.Disconnect)(self.dbc);
            (self.lib.FreeHandle)(SQL_HANDLE_DBC, self.dbc);
        }
    }
}

struct Stmt<'a> {
    conn: &'a OdbcConn,
    handle: Handle,
}

impl Drop for Stmt<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.conn.lib.FreeHandle)(SQL_HANDLE_STMT, self.handle);
        }
    }
}

fn sql_type_name(t: i16) -> &'static str {
    match t {
        1 => "char",
        2 | 3 => "decimal",
        4 => "integer",
        5 => "smallint",
        6 | 8 => "double",
        7 => "real",
        9 | 91 => "date",
        10 | 92 => "time",
        11 | 93 => "timestamp",
        12 => "varchar",
        -1 => "longvarchar",
        -2 => "binary",
        -3 => "varbinary",
        -4 => "longvarbinary",
        -5 => "bigint",
        -6 => "tinyint",
        -7 => "bit",
        -8 => "nchar",
        -9 => "nvarchar",
        -10 => "longnvarchar",
        -11 => "guid",
        _ => "",
    }
}

impl DbConnection for OdbcConn {
    fn dialect(&self) -> Dialect {
        Dialect::Generic
    }

    fn execute(
        &mut self,
        sql: &str,
        cancel: &AtomicBool,
        on_row: &mut RowSink,
    ) -> AppResult<ExecMeta> {
        let stmt = self.alloc_stmt()?;
        let sql_enc = encode(sql);
        let r = unsafe { (self.lib.ExecDirect)(stmt.handle, sql_enc.as_ptr(), SQL_NTS) };
        if !(ok(r) || r == SQL_NO_DATA) {
            return Err(self.lib.diag(SQL_HANDLE_STMT, stmt.handle, "execute"));
        }
        self.read_result(&stmt, cancel, on_row)
    }

    fn catalog(&mut self) -> AppResult<Catalog> {
        let none = AtomicBool::new(false);

        // All tables & views in one call.
        let stmt = self.alloc_stmt()?;
        let types = encode("TABLE,VIEW");
        let r = unsafe {
            (self.lib.Tables)(
                stmt.handle,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                types.as_ptr(),
                types.len() as i16,
            )
        };
        if !ok(r) {
            return Err(self.lib.diag(SQL_HANDLE_STMT, stmt.handle, "SQLTables"));
        }
        let mut tables_rows: Vec<Vec<DbValue>> = Vec::new();
        let mut sink = |ev: RowEvent| {
            if let RowEvent::Row(row) = ev {
                tables_rows.push(row);
            }
            ControlFlow::Continue(())
        };
        self.read_result(&stmt, &none, &mut sink)?;
        drop(stmt);

        let mut schemas: BTreeMap<String, BTreeMap<String, TableInfo>> = BTreeMap::new();
        for row in &tables_rows {
            let cat = txt(row, 0);
            let sch = txt(row, 1);
            let name = txt(row, 2);
            let ty = txt(row, 3);
            if name.is_empty() {
                continue;
            }
            let key = schema_key(&cat, &sch);
            let kind = if ty.to_ascii_uppercase().contains("VIEW") { "view" } else { "table" };
            schemas
                .entry(key)
                .or_default()
                .insert(name.clone(), TableInfo::new(name, kind));
        }

        // All columns in one call.
        let stmt = self.alloc_stmt()?;
        let r = unsafe {
            (self.lib.Columns)(
                stmt.handle,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
            )
        };
        if ok(r) {
            let mut col_rows: Vec<Vec<DbValue>> = Vec::new();
            let mut sink = |ev: RowEvent| {
                if let RowEvent::Row(row) = ev {
                    col_rows.push(row);
                }
                ControlFlow::Continue(())
            };
            self.read_result(&stmt, &none, &mut sink)?;
            // SQLColumns result: 0 cat, 1 schem, 2 table, 3 column, 4 type,
            // 5 type_name, 6 size, ... 10 nullable, 12 default
            for row in &col_rows {
                let key = schema_key(&txt(row, 0), &txt(row, 1));
                let tbl = txt(row, 2);
                if let Some(t) = schemas.get_mut(&key).and_then(|s| s.get_mut(&tbl)) {
                    let mut data_type = txt(row, 5);
                    let size = txt(row, 6);
                    if !size.is_empty()
                        && matches!(
                            data_type.to_ascii_lowercase().as_str(),
                            "varchar" | "char" | "nvarchar" | "nchar" | "varbinary"
                        )
                    {
                        data_type = format!("{data_type}({size})");
                    }
                    t.columns.push(ColumnInfo {
                        name: txt(row, 3),
                        data_type,
                        nullable: txt(row, 10) != "0",
                        default: {
                            let d = txt(row, 12);
                            if d.is_empty() { None } else { Some(d) }
                        },
                    });
                }
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

fn txt(row: &[DbValue], i: usize) -> String {
    match row.get(i) {
        Some(DbValue::Text(s)) => s.clone(),
        Some(DbValue::Null) | None => String::new(),
        Some(v) => v.as_text(),
    }
}
