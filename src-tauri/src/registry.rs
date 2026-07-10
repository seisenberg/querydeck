use std::io::{Read, Write};

use serde::Serialize;
use tauri::Emitter;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// A downloadable driver package (JDBC jars from Maven Central).
#[derive(Debug, Clone, Serialize)]
pub struct DriverPackage {
    pub id: &'static str,
    pub title: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    pub filename: &'static str,
    #[serde(rename = "driverClass")]
    pub driver_class: &'static str,
    #[serde(rename = "urlTemplate")]
    pub url_template: &'static str,
    #[serde(rename = "defaultPort")]
    pub default_port: u16,
    pub notes: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct DriverPackageStatus {
    #[serde(flatten)]
    pub package: DriverPackage,
    pub downloaded: bool,
    pub path: Option<String>,
}

pub const PACKAGES: &[DriverPackage] = &[
    DriverPackage {
        id: "jdbc-postgresql",
        title: "PostgreSQL (JDBC)",
        version: "42.7.7",
        url: "https://repo1.maven.org/maven2/org/postgresql/postgresql/42.7.7/postgresql-42.7.7.jar",
        filename: "postgresql-42.7.7.jar",
        driver_class: "org.postgresql.Driver",
        url_template: "jdbc:postgresql://{host}:{port}/{database}",
        default_port: 5432,
        notes: "Native driver is also built in; JDBC is optional for Postgres.",
    },
    DriverPackage {
        id: "jdbc-mysql",
        title: "MySQL Connector/J",
        version: "9.3.0",
        url: "https://repo1.maven.org/maven2/com/mysql/mysql-connector-j/9.3.0/mysql-connector-j-9.3.0.jar",
        filename: "mysql-connector-j-9.3.0.jar",
        driver_class: "com.mysql.cj.jdbc.Driver",
        url_template: "jdbc:mysql://{host}:{port}/{database}",
        default_port: 3306,
        notes: "Native driver is also built in; JDBC is optional for MySQL.",
    },
    DriverPackage {
        id: "jdbc-mariadb",
        title: "MariaDB",
        version: "3.5.3",
        url: "https://repo1.maven.org/maven2/org/mariadb/jdbc/mariadb-java-client/3.5.3/mariadb-java-client-3.5.3.jar",
        filename: "mariadb-java-client-3.5.3.jar",
        driver_class: "org.mariadb.jdbc.Driver",
        url_template: "jdbc:mariadb://{host}:{port}/{database}",
        default_port: 3306,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-sqlite",
        title: "SQLite (Xerial JDBC)",
        version: "3.49.1.0",
        url: "https://repo1.maven.org/maven2/org/xerial/sqlite-jdbc/3.49.1.0/sqlite-jdbc-3.49.1.0.jar",
        filename: "sqlite-jdbc-3.49.1.0.jar",
        driver_class: "org.sqlite.JDBC",
        url_template: "jdbc:sqlite:{file}",
        default_port: 0,
        notes: "Native driver is also built in; JDBC is optional for SQLite.",
    },
    DriverPackage {
        id: "jdbc-duckdb",
        title: "DuckDB",
        version: "1.3.0.0",
        url: "https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/1.3.0.0/duckdb_jdbc-1.3.0.0.jar",
        filename: "duckdb_jdbc-1.3.0.0.jar",
        driver_class: "org.duckdb.DuckDBDriver",
        url_template: "jdbc:duckdb:{file}",
        default_port: 0,
        notes: "Use an empty file path for an in-memory database.",
    },
    DriverPackage {
        id: "jdbc-clickhouse",
        title: "ClickHouse",
        version: "0.6.5",
        url: "https://repo1.maven.org/maven2/com/clickhouse/clickhouse-jdbc/0.6.5/clickhouse-jdbc-0.6.5-shaded.jar",
        filename: "clickhouse-jdbc-0.6.5-shaded.jar",
        driver_class: "com.clickhouse.jdbc.ClickHouseDriver",
        url_template: "jdbc:clickhouse://{host}:{port}/{database}",
        default_port: 8123,
        notes: "Shaded jar (self-contained).",
    },
    DriverPackage {
        id: "jdbc-mssql",
        title: "Microsoft SQL Server",
        version: "12.10.0.jre11",
        url: "https://repo1.maven.org/maven2/com/microsoft/sqlserver/mssql-jdbc/12.10.0.jre11/mssql-jdbc-12.10.0.jre11.jar",
        filename: "mssql-jdbc-12.10.0.jre11.jar",
        driver_class: "com.microsoft.sqlserver.jdbc.SQLServerDriver",
        url_template: "jdbc:sqlserver://{host}:{port};databaseName={database};encrypt=true;trustServerCertificate=true",
        default_port: 1433,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-oracle",
        title: "Oracle Database",
        version: "23.8.0.25.04",
        url: "https://repo1.maven.org/maven2/com/oracle/database/jdbc/ojdbc11/23.8.0.25.04/ojdbc11-23.8.0.25.04.jar",
        filename: "ojdbc11-23.8.0.25.04.jar",
        driver_class: "oracle.jdbc.OracleDriver",
        url_template: "jdbc:oracle:thin:@//{host}:{port}/{service}",
        default_port: 1521,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-h2",
        title: "H2",
        version: "2.3.232",
        url: "https://repo1.maven.org/maven2/com/h2database/h2/2.3.232/h2-2.3.232.jar",
        filename: "h2-2.3.232.jar",
        driver_class: "org.h2.Driver",
        url_template: "jdbc:h2:{file}",
        default_port: 0,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-trino",
        title: "Trino",
        version: "476",
        url: "https://repo1.maven.org/maven2/io/trino/trino-jdbc/476/trino-jdbc-476.jar",
        filename: "trino-jdbc-476.jar",
        driver_class: "io.trino.jdbc.TrinoDriver",
        url_template: "jdbc:trino://{host}:{port}/{catalog}",
        default_port: 8080,
        notes: "",
    },
];

pub fn list() -> Vec<DriverPackageStatus> {
    let dir = AppState::drivers_dir();
    PACKAGES
        .iter()
        .map(|p| {
            let path = dir.join(p.filename);
            let downloaded = path.exists();
            DriverPackageStatus {
                package: p.clone(),
                downloaded,
                path: if downloaded {
                    Some(path.to_string_lossy().into_owned())
                } else {
                    None
                },
            }
        })
        .collect()
}

#[derive(Debug, Serialize, Clone)]
struct DownloadProgress {
    id: String,
    done: u64,
    total: u64,
}

pub fn download(app: &tauri::AppHandle, id: &str) -> AppResult<String> {
    let pkg = PACKAGES
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::config(format!("unknown driver package {id}")))?;

    let dir = AppState::drivers_dir();
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(pkg.filename);
    let tmp = dir.join(format!("{}.part", pkg.filename));

    let mut resp = reqwest::blocking::get(pkg.url)
        .map_err(|e| AppError::io(format!("download failed: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::io(format!("download failed: {e}")))?;
    let total = resp.content_length().unwrap_or(0);

    let mut file = std::fs::File::create(&tmp)?;
    let mut buf = [0u8; 64 * 1024];
    let mut done: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    loop {
        let n = resp
            .read(&mut buf)
            .map_err(|e| AppError::io(format!("download failed: {e}")))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if last_emit.elapsed().as_millis() > 100 {
            last_emit = std::time::Instant::now();
            let _ = app.emit(
                "driver:progress",
                DownloadProgress { id: id.to_string(), done, total },
            );
        }
    }
    file.flush()?;
    drop(file);

    // Sanity check: jars are zip files.
    let head = std::fs::read(&tmp)?;
    if head.len() < 4 || &head[..2] != b"PK" {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::io("downloaded file is not a jar (bad content)"));
    }
    std::fs::rename(&tmp, &dest)?;
    let _ = app.emit(
        "driver:progress",
        DownloadProgress { id: id.to_string(), done, total: done },
    );
    Ok(dest.to_string_lossy().into_owned())
}
