use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::ops::ControlFlow;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::drivers::{DbConnection, Dialect, RowEvent};
use crate::error::{AppError, AppResult};
use crate::value::{ColumnMeta, DbValue};

// ---------------------------------------------------------------- preview --

#[derive(Debug, Serialize)]
pub struct ImportPreview {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub delimiter: String,
    #[serde(rename = "hasHeader")]
    pub has_header: bool,
    #[serde(rename = "totalBytes")]
    pub total_bytes: u64,
    /// Suggested column types inferred from the sample: "bigint" | "double" | "boolean" | "text"
    #[serde(rename = "inferredTypes")]
    pub inferred_types: Vec<String>,
}

pub fn import_preview(
    path: &str,
    delimiter: Option<String>,
    has_header: Option<bool>,
) -> AppResult<ImportPreview> {
    let total_bytes = std::fs::metadata(path)?.len();
    let mut head = vec![0u8; 128 * 1024];
    let mut f = File::open(path)?;
    let n = f.read(&mut head)?;
    head.truncate(n);
    let sample = String::from_utf8_lossy(&head);

    let delim = match delimiter.as_deref().and_then(|d| d.chars().next()) {
        Some(d) => d,
        None => sniff_delimiter(&sample),
    };

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delim as u8)
        .has_headers(false)
        .flexible(true)
        .from_path(path)
        .map_err(|e| AppError::io(e.to_string()))?;

    let mut records: Vec<Vec<String>> = Vec::new();
    for rec in rdr.records().take(101) {
        let rec = rec.map_err(|e| AppError::io(e.to_string()))?;
        records.push(rec.iter().map(|s| s.to_string()).collect());
    }
    if records.is_empty() {
        return Err(AppError::io("file is empty"));
    }

    let has_header = has_header.unwrap_or_else(|| {
        // Header heuristic: first row has no purely numeric / empty cells.
        records[0]
            .iter()
            .all(|c| !c.trim().is_empty() && c.parse::<f64>().is_err())
    });

    let ncols = records.iter().map(|r| r.len()).max().unwrap_or(0);
    let columns: Vec<String> = if has_header {
        (0..ncols)
            .map(|i| {
                records[0]
                    .get(i)
                    .filter(|s| !s.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("column_{}", i + 1))
            })
            .collect()
    } else {
        (0..ncols).map(|i| format!("column_{}", i + 1)).collect()
    };

    let data_rows: Vec<Vec<String>> = records
        .iter()
        .skip(if has_header { 1 } else { 0 })
        .take(50)
        .cloned()
        .collect();

    let inferred_types = (0..ncols)
        .map(|i| infer_type(data_rows.iter().filter_map(|r| r.get(i).map(|s| s.as_str()))))
        .collect();

    Ok(ImportPreview {
        columns,
        rows: data_rows,
        delimiter: delim.to_string(),
        has_header,
        total_bytes,
        inferred_types,
    })
}

fn sniff_delimiter(sample: &str) -> char {
    let lines: Vec<&str> = sample.lines().take(10).collect();
    let mut best = (',', 0usize);
    for cand in [',', ';', '\t', '|'] {
        let counts: Vec<usize> = lines.iter().map(|l| l.matches(cand).count()).collect();
        let min = counts.iter().min().copied().unwrap_or(0);
        if min > 0 && min >= best.1 {
            best = (cand, min);
        }
    }
    best.0
}

fn infer_type<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let mut saw_any = false;
    let mut all_int = true;
    let mut all_float = true;
    let mut all_bool = true;
    for v in values {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        saw_any = true;
        if v.parse::<i64>().is_err() {
            all_int = false;
        }
        if v.parse::<f64>().is_err() {
            all_float = false;
        }
        if !matches!(v.to_ascii_lowercase().as_str(), "true" | "false" | "t" | "f" | "0" | "1") {
            all_bool = false;
        }
        if !all_int && !all_float && !all_bool {
            break;
        }
    }
    if !saw_any {
        return "text".into();
    }
    if all_bool && !all_int {
        return "boolean".into();
    }
    if all_int {
        return "bigint".into();
    }
    if all_float {
        return "double".into();
    }
    "text".into()
}

/// Map an abstract inferred type to a dialect column type.
pub fn dialect_type(dialect: Dialect, abstract_type: &str) -> String {
    match (dialect, abstract_type) {
        (Dialect::Postgres, "double") => "double precision".into(),
        (Dialect::Mysql, "boolean") => "tinyint(1)".into(),
        (Dialect::Sqlite, "bigint") => "INTEGER".into(),
        (Dialect::Sqlite, "double") => "REAL".into(),
        (Dialect::Sqlite, _) if abstract_type == "text" => "TEXT".into(),
        (Dialect::Generic, "double") => "double precision".into(),
        (Dialect::Generic, "text") => "varchar(4000)".into(),
        _ => abstract_type.into(),
    }
}

// ----------------------------------------------------------------- import --

#[derive(Debug, Deserialize)]
pub struct ImportMapping {
    /// Index of the source column in the file.
    #[serde(rename = "fileIndex")]
    pub file_index: usize,
    /// Target table column name.
    pub column: String,
    /// Column type when creating a new table (abstract or dialect type).
    #[serde(rename = "dataType")]
    pub data_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportRequest {
    #[serde(rename = "profileId")]
    pub profile_id: String,
    #[serde(rename = "jobId")]
    pub job_id: String,
    pub path: String,
    pub delimiter: String,
    #[serde(rename = "hasHeader")]
    pub has_header: bool,
    pub schema: String,
    pub table: String,
    #[serde(rename = "createTable")]
    pub create_table: bool,
    pub mappings: Vec<ImportMapping>,
    #[serde(rename = "emptyAsNull", default = "default_true")]
    pub empty_as_null: bool,
    #[serde(rename = "batchSize", default = "default_batch")]
    pub batch_size: usize,
}

fn default_true() -> bool {
    true
}
fn default_batch() -> usize {
    200
}

#[derive(Debug, Serialize, Clone)]
pub struct JobProgress {
    #[serde(rename = "jobId")]
    pub job_id: String,
    #[serde(rename = "rowsDone")]
    pub rows_done: u64,
    #[serde(rename = "bytesDone")]
    pub bytes_done: u64,
    #[serde(rename = "totalBytes")]
    pub total_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    #[serde(rename = "rowsImported")]
    pub rows_imported: u64,
    #[serde(rename = "elapsedMs")]
    pub elapsed_ms: u64,
}

fn exec_simple(conn: &mut Box<dyn DbConnection>, sql: &str) -> AppResult<u64> {
    let none = AtomicBool::new(false);
    let mut sink = |_ev: RowEvent| ControlFlow::Continue(());
    let meta = conn.execute(sql, &none, &mut sink)?;
    Ok(meta.affected.unwrap_or(0))
}

fn qualified_name(dialect: Dialect, schema: &str, table: &str) -> String {
    if schema.is_empty() || schema == "main" || schema == "(default)" {
        dialect.quote_ident(table)
    } else {
        format!("{}.{}", dialect.quote_ident(schema), dialect.quote_ident(table))
    }
}

pub fn import_run(
    app: &tauri::AppHandle,
    conn: &mut Box<dyn DbConnection>,
    req: &ImportRequest,
    cancel: &AtomicBool,
) -> AppResult<ImportResult> {
    let start = std::time::Instant::now();
    let dialect = conn.dialect();
    let total_bytes = std::fs::metadata(&req.path)?.len();
    let target = qualified_name(dialect, &req.schema, &req.table);

    if req.create_table {
        let cols: Vec<String> = req
            .mappings
            .iter()
            .map(|m| {
                let ty = m
                    .data_type
                    .clone()
                    .unwrap_or_else(|| "text".into());
                let ty = dialect_type(dialect, &ty);
                format!("{} {}", dialect.quote_ident(&m.column), ty)
            })
            .collect();
        exec_simple(conn, &format!("CREATE TABLE {target} ({})", cols.join(", ")))?;
    }

    let delim = req.delimiter.chars().next().unwrap_or(',') as u8;
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(req.has_header)
        .flexible(true)
        .from_path(&req.path)
        .map_err(|e| AppError::io(e.to_string()))?;

    let col_list: Vec<String> = req
        .mappings
        .iter()
        .map(|m| dialect.quote_ident(&m.column))
        .collect();
    let insert_head = format!("INSERT INTO {target} ({}) VALUES ", col_list.join(", "));

    let txn = matches!(dialect, Dialect::Sqlite | Dialect::Postgres | Dialect::Mysql);
    if txn {
        exec_simple(conn, "BEGIN")?;
    }

    let mut batch: Vec<String> = Vec::with_capacity(req.batch_size);
    let mut rows_done: u64 = 0;
    let mut last_emit = std::time::Instant::now();

    let flush = |conn: &mut Box<dyn DbConnection>, batch: &mut Vec<String>| -> AppResult<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let sql = format!("{insert_head}{}", batch.join(", "));
        exec_simple(conn, &sql)?;
        batch.clear();
        Ok(())
    };

    let run = (|| -> AppResult<u64> {
        for rec in rdr.records() {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(AppError::cancelled());
            }
            let rec = rec.map_err(|e| AppError::io(e.to_string()))?;
            let mut tuple: Vec<String> = Vec::with_capacity(req.mappings.len());
            for m in &req.mappings {
                let raw = rec.get(m.file_index).unwrap_or("");
                let lit = if raw.is_empty() && req.empty_as_null {
                    "NULL".to_string()
                } else {
                    dialect.literal(&DbValue::Text(raw.to_string()))
                };
                tuple.push(lit);
            }
            batch.push(format!("({})", tuple.join(", ")));
            rows_done += 1;

            if batch.len() >= req.batch_size {
                flush(conn, &mut batch)?;
                if last_emit.elapsed().as_millis() > 100 {
                    last_emit = std::time::Instant::now();
                    let _ = app.emit(
                        "job:progress",
                        JobProgress {
                            job_id: req.job_id.clone(),
                            rows_done,
                            bytes_done: rec.position().map(|p| p.byte()).unwrap_or(0),
                            total_bytes,
                        },
                    );
                }
            }
        }
        flush(conn, &mut batch)?;
        Ok(rows_done)
    })();

    match run {
        Ok(n) => {
            if txn {
                exec_simple(conn, "COMMIT")?;
            }
            Ok(ImportResult { rows_imported: n, elapsed_ms: start.elapsed().as_millis() as u64 })
        }
        Err(e) => {
            if txn {
                let _ = exec_simple(conn, "ROLLBACK");
            }
            Err(e)
        }
    }
}

// ----------------------------------------------------------------- export --

#[derive(Debug, Deserialize)]
pub struct ExportRequest {
    #[serde(rename = "profileId")]
    pub profile_id: String,
    #[serde(rename = "jobId")]
    pub job_id: String,
    pub sql: String,
    /// "csv" | "tsv" | "json" | "sql" | "xlsx"
    pub format: String,
    pub path: String,
    #[serde(rename = "includeHeader", default = "default_true")]
    pub include_header: bool,
    /// Row cap; 0 = everything.
    #[serde(default)]
    pub limit: usize,
    /// Target table name used for INSERT/CREATE statements in sql format.
    #[serde(rename = "tableName", default)]
    pub table_name: String,
    /// Per-tab database override so the export runs against the same
    /// database the console is pointed at.
    #[serde(default)]
    pub database: Option<String>,
    /// sql format: emit a CREATE TABLE statement first.
    #[serde(rename = "includeCreate", default)]
    pub include_create: bool,
    /// sql format: "single" (one INSERT per row) | "multiline" (multi-row VALUES).
    #[serde(rename = "insertStyle", default = "default_insert_style")]
    pub insert_style: String,
    /// sql multiline: start a new INSERT every N rows (0 = one statement).
    #[serde(rename = "batchLimit", default)]
    pub batch_limit: usize,
}

fn default_insert_style() -> String {
    "single".into()
}

#[derive(Debug, Serialize)]
pub struct ExportResult {
    #[serde(rename = "rowsExported")]
    pub rows_exported: u64,
    #[serde(rename = "elapsedMs")]
    pub elapsed_ms: u64,
}

enum ExportSink {
    Csv { w: csv::Writer<BufWriter<File>>, include_header: bool },
    Json { w: BufWriter<File>, first: bool },
    Sql {
        w: BufWriter<File>,
        dialect: Dialect,
        table: String,
        col_list: String,
        include_create: bool,
        multiline: bool,
        batch_limit: usize,
        rows_in_batch: usize,
    },
    Xlsx {
        workbook: rust_xlsxwriter::Workbook,
        path: String,
        include_header: bool,
        row: u32,
    },
}

impl ExportSink {
    fn columns(&mut self, cols: &[ColumnMeta]) -> std::io::Result<()> {
        match self {
            ExportSink::Csv { w, include_header } => {
                if *include_header {
                    w.write_record(cols.iter().map(|c| c.name.as_str()))
                        .map_err(std::io::Error::other)?;
                }
            }
            ExportSink::Json { w, .. } => {
                w.write_all(b"[")?;
            }
            ExportSink::Sql { w, dialect, table, col_list, include_create, .. } => {
                let names: Vec<String> =
                    cols.iter().map(|c| dialect.quote_ident(&c.name)).collect();
                *col_list = names.join(", ");
                if *include_create {
                    let col_defs: Vec<String> = cols
                        .iter()
                        .map(|c| {
                            let ty = if c.type_name.trim().is_empty() {
                                "TEXT"
                            } else {
                                c.type_name.as_str()
                            };
                            format!("    {} {}", dialect.quote_ident(&c.name), ty)
                        })
                        .collect();
                    writeln!(w, "CREATE TABLE {table} (\n{}\n);\n", col_defs.join(",\n"))?;
                }
            }
            ExportSink::Xlsx { workbook, include_header, row, .. } => {
                if *include_header {
                    let ws = workbook.worksheet_from_index(0).map_err(std::io::Error::other)?;
                    for (i, c) in cols.iter().enumerate() {
                        ws.write_string(*row, i as u16, &c.name).map_err(std::io::Error::other)?;
                    }
                    *row += 1;
                }
            }
        }
        Ok(())
    }

    fn row(&mut self, values: &[DbValue], cols: &[ColumnMeta]) -> std::io::Result<()> {
        match self {
            ExportSink::Csv { w, .. } => {
                w.write_record(values.iter().map(|v| v.as_text()))
                    .map_err(std::io::Error::other)?;
            }
            ExportSink::Json { w, first } => {
                let obj: serde_json::Map<String, serde_json::Value> = cols
                    .iter()
                    .zip(values.iter())
                    .map(|(c, v)| (c.name.clone(), serde_json::to_value(v).unwrap_or_default()))
                    .collect();
                let sep: &[u8] = if *first { b"\n  " } else { b",\n  " };
                *first = false;
                w.write_all(sep)?;
                serde_json::to_writer(&mut *w, &obj).map_err(std::io::Error::other)?;
            }
            ExportSink::Sql {
                w,
                dialect,
                table,
                col_list,
                multiline,
                batch_limit,
                rows_in_batch,
                ..
            } => {
                let vals: Vec<String> = values.iter().map(|v| dialect.literal(v)).collect();
                let tuple = format!("({})", vals.join(", "));
                if !*multiline {
                    writeln!(w, "INSERT INTO {table} ({col_list}) VALUES {tuple};")?;
                } else {
                    if *rows_in_batch == 0 {
                        write!(w, "INSERT INTO {table} ({col_list}) VALUES\n{tuple}")?;
                    } else {
                        write!(w, ",\n{tuple}")?;
                    }
                    *rows_in_batch += 1;
                    if *batch_limit > 0 && *rows_in_batch >= *batch_limit {
                        w.write_all(b";\n\n")?;
                        *rows_in_batch = 0;
                    }
                }
            }
            ExportSink::Xlsx { workbook, row, .. } => {
                let ws = workbook.worksheet_from_index(0).map_err(std::io::Error::other)?;
                for (i, v) in values.iter().enumerate() {
                    let col = i as u16;
                    match v {
                        DbValue::Null => {}
                        DbValue::Bool(b) => {
                            ws.write_boolean(*row, col, *b).map_err(std::io::Error::other)?;
                        }
                        DbValue::Int(n) => {
                            ws.write_number(*row, col, *n as f64)
                                .map_err(std::io::Error::other)?;
                        }
                        DbValue::Float(f) => {
                            ws.write_number(*row, col, *f).map_err(std::io::Error::other)?;
                        }
                        DbValue::Text(s) => {
                            // Numeric-looking text (e.g. Postgres simple-protocol
                            // values) becomes a real number in the sheet.
                            if let Ok(n) = s.parse::<f64>() {
                                if s.len() < 18 {
                                    ws.write_number(*row, col, n)
                                        .map_err(std::io::Error::other)?;
                                    continue;
                                }
                            }
                            ws.write_string(*row, col, s).map_err(std::io::Error::other)?;
                        }
                    }
                }
                *row += 1;
            }
        }
        Ok(())
    }

    fn finish(self) -> std::io::Result<()> {
        match self {
            ExportSink::Csv { mut w, .. } => {
                w.flush()?;
            }
            ExportSink::Json { mut w, first } => {
                if first {
                    w.write_all(b"]\n")?;
                } else {
                    w.write_all(b"\n]\n")?;
                }
                w.flush()?;
            }
            ExportSink::Sql { mut w, multiline, rows_in_batch, .. } => {
                if multiline && rows_in_batch > 0 {
                    w.write_all(b";\n")?;
                }
                w.flush()?;
            }
            ExportSink::Xlsx { mut workbook, path, .. } => {
                workbook.save(&path).map_err(std::io::Error::other)?;
            }
        }
        Ok(())
    }
}

pub fn export_run(
    app: &tauri::AppHandle,
    conn: &mut Box<dyn DbConnection>,
    req: &ExportRequest,
    cancel: &AtomicBool,
) -> AppResult<ExportResult> {
    let start = std::time::Instant::now();
    let dialect = conn.dialect();

    let mut sink = match req.format.as_str() {
        "csv" | "tsv" => ExportSink::Csv {
            w: csv::WriterBuilder::new()
                .delimiter(if req.format == "tsv" { b'\t' } else { b',' })
                .from_writer(BufWriter::new(File::create(&req.path)?)),
            include_header: req.include_header,
        },
        "json" => ExportSink::Json { w: BufWriter::new(File::create(&req.path)?), first: true },
        "sql" => ExportSink::Sql {
            w: BufWriter::new(File::create(&req.path)?),
            dialect,
            table: if req.table_name.is_empty() {
                "exported_table".into()
            } else {
                req.table_name.clone()
            },
            col_list: String::new(),
            include_create: req.include_create,
            multiline: req.insert_style == "multiline",
            batch_limit: req.batch_limit,
            rows_in_batch: 0,
        },
        "xlsx" => {
            let mut workbook = rust_xlsxwriter::Workbook::new();
            workbook.add_worksheet();
            ExportSink::Xlsx {
                workbook,
                path: req.path.clone(),
                include_header: req.include_header,
                row: 0,
            }
        }
        other => return Err(AppError::config(format!("unknown export format {other}"))),
    };

    let mut rows_done: u64 = 0;
    let mut columns: Vec<ColumnMeta> = Vec::new();
    let mut io_error: Option<AppError> = None;
    let mut last_emit = std::time::Instant::now();

    let mut on_event = |ev: RowEvent| -> ControlFlow<()> {
        let res: std::io::Result<()> = match ev {
            RowEvent::Columns(cols) => {
                columns = cols;
                sink.columns(&columns)
            }
            RowEvent::Row(row) => {
                if req.limit > 0 && rows_done >= req.limit as u64 {
                    return ControlFlow::Break(());
                }
                let r = sink.row(&row, &columns);
                if r.is_ok() {
                    rows_done += 1;
                    if last_emit.elapsed().as_millis() > 100 {
                        last_emit = std::time::Instant::now();
                        let _ = app.emit(
                            "job:progress",
                            JobProgress {
                                job_id: req.job_id.clone(),
                                rows_done,
                                bytes_done: 0,
                                total_bytes: 0,
                            },
                        );
                    }
                }
                r
            }
        };
        match res {
            Ok(()) => ControlFlow::Continue(()),
            Err(e) => {
                io_error = Some(AppError::io(e.to_string()));
                ControlFlow::Break(())
            }
        }
    };

    let meta = conn.execute(&req.sql, cancel, &mut on_event)?;
    drop(on_event);

    if let Some(e) = io_error {
        return Err(e);
    }
    if meta.columns.is_none() {
        return Err(AppError::query("statement did not return a result set"));
    }
    sink.finish()?;

    Ok(ExportResult { rows_exported: rows_done, elapsed_ms: start.elapsed().as_millis() as u64 })
}
