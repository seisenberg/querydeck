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

/// One entry per engine, latest stable version from the vendor's official
/// Maven Central coordinates (all URLs verified resolvable). Only
/// self-contained (single-jar) drivers are listed — a download that then
/// fails on missing transitive jars is worse than no button.
pub const PACKAGES: &[DriverPackage] = &[
    DriverPackage {
        id: "jdbc-postgresql",
        title: "PostgreSQL (JDBC)",
        version: "42.7.13",
        url: "https://repo1.maven.org/maven2/org/postgresql/postgresql/42.7.13/postgresql-42.7.13.jar",
        filename: "postgresql-42.7.13.jar",
        driver_class: "org.postgresql.Driver",
        url_template: "jdbc:postgresql://{host}:{port}/{database}",
        default_port: 5432,
        notes: "Native driver is built in; also works for CockroachDB, Greenplum and YugabyteDB.",
    },
    DriverPackage {
        id: "jdbc-mysql",
        title: "MySQL Connector/J",
        version: "9.7.0",
        url: "https://repo1.maven.org/maven2/com/mysql/mysql-connector-j/9.7.0/mysql-connector-j-9.7.0.jar",
        filename: "mysql-connector-j-9.7.0.jar",
        driver_class: "com.mysql.cj.jdbc.Driver",
        url_template: "jdbc:mysql://{host}:{port}/{database}",
        default_port: 3306,
        notes: "Native driver is built in; also works for TiDB and SingleStore.",
    },
    DriverPackage {
        id: "jdbc-mariadb",
        title: "MariaDB",
        version: "3.5.9",
        url: "https://repo1.maven.org/maven2/org/mariadb/jdbc/mariadb-java-client/3.5.9/mariadb-java-client-3.5.9.jar",
        filename: "mariadb-java-client-3.5.9.jar",
        driver_class: "org.mariadb.jdbc.Driver",
        url_template: "jdbc:mariadb://{host}:{port}/{database}",
        default_port: 3306,
        notes: "Native driver is built in.",
    },
    DriverPackage {
        id: "jdbc-sqlite",
        title: "SQLite (Xerial JDBC)",
        version: "3.53.2.0",
        url: "https://repo1.maven.org/maven2/org/xerial/sqlite-jdbc/3.53.2.0/sqlite-jdbc-3.53.2.0.jar",
        filename: "sqlite-jdbc-3.53.2.0.jar",
        driver_class: "org.sqlite.JDBC",
        url_template: "jdbc:sqlite:{file}",
        default_port: 0,
        notes: "Native driver is built in; JDBC is optional for SQLite.",
    },
    DriverPackage {
        id: "jdbc-duckdb",
        title: "DuckDB",
        version: "1.5.4.0",
        url: "https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/1.5.4.0/duckdb_jdbc-1.5.4.0.jar",
        filename: "duckdb_jdbc-1.5.4.0.jar",
        driver_class: "org.duckdb.DuckDBDriver",
        url_template: "jdbc:duckdb:{file}",
        default_port: 0,
        notes: "Use an empty file path for an in-memory database.",
    },
    DriverPackage {
        id: "jdbc-clickhouse",
        title: "ClickHouse (JDBC)",
        version: "0.9.8",
        url: "https://repo1.maven.org/maven2/com/clickhouse/clickhouse-jdbc/0.9.8/clickhouse-jdbc-0.9.8-all.jar",
        filename: "clickhouse-jdbc-0.9.8-all.jar",
        driver_class: "com.clickhouse.jdbc.ClickHouseDriver",
        url_template: "jdbc:clickhouse://{host}:{port}/{database}",
        default_port: 8123,
        notes: "Native (HTTP) driver is built in; shaded jar, self-contained.",
    },
    DriverPackage {
        id: "jdbc-mssql",
        title: "Microsoft SQL Server (JDBC)",
        version: "13.4.0.jre11",
        url: "https://repo1.maven.org/maven2/com/microsoft/sqlserver/mssql-jdbc/13.4.0.jre11/mssql-jdbc-13.4.0.jre11.jar",
        filename: "mssql-jdbc-13.4.0.jre11.jar",
        driver_class: "com.microsoft.sqlserver.jdbc.SQLServerDriver",
        url_template: "jdbc:sqlserver://{host}:{port};databaseName={database};encrypt=true;trustServerCertificate=true",
        default_port: 1433,
        notes: "Native (TDS) driver is built in; also works for Azure SQL.",
    },
    DriverPackage {
        id: "jdbc-oracle",
        title: "Oracle Database",
        version: "23.26.2.0.0",
        url: "https://repo1.maven.org/maven2/com/oracle/database/jdbc/ojdbc11/23.26.2.0.0/ojdbc11-23.26.2.0.0.jar",
        filename: "ojdbc11-23.26.2.0.0.jar",
        driver_class: "oracle.jdbc.OracleDriver",
        url_template: "jdbc:oracle:thin:@//{host}:{port}/{service}",
        default_port: 1521,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-snowflake",
        title: "Snowflake",
        version: "4.3.1",
        url: "https://repo1.maven.org/maven2/net/snowflake/snowflake-jdbc/4.3.1/snowflake-jdbc-4.3.1.jar",
        filename: "snowflake-jdbc-4.3.1.jar",
        driver_class: "net.snowflake.client.jdbc.SnowflakeDriver",
        url_template: "jdbc:snowflake://{account}.snowflakecomputing.com/?db={database}&warehouse={warehouse}",
        default_port: 443,
        notes: "Account is the org-account identifier, e.g. myorg-myaccount.",
    },
    DriverPackage {
        id: "jdbc-redshift",
        title: "Amazon Redshift (JDBC)",
        version: "2.2.7",
        url: "https://repo1.maven.org/maven2/com/amazon/redshift/redshift-jdbc42/2.2.7/redshift-jdbc42-2.2.7.jar",
        filename: "redshift-jdbc42-2.2.7.jar",
        driver_class: "com.amazon.redshift.Driver",
        url_template: "jdbc:redshift://{host}:{port}/{database}",
        default_port: 5439,
        notes: "Native (PG wire) driver is built in; JDBC adds IAM auth options.",
    },
    DriverPackage {
        id: "jdbc-athena",
        title: "Amazon Athena",
        version: "2026.24.1",
        url: "https://repo1.maven.org/maven2/com/amazonaws/athena-jdbc/2026.24.1/athena-jdbc-2026.24.1.jar",
        filename: "athena-jdbc-2026.24.1.jar",
        driver_class: "com.amazonaws.athena.jdbc.AthenaDriver",
        url_template: "jdbc:athena://Region={region};WorkGroup=primary;",
        default_port: 443,
        notes: "Uses AWS credentials (default chain or User/Password as key/secret).",
    },
    DriverPackage {
        id: "jdbc-db2",
        title: "IBM Db2",
        version: "12.1.5.0",
        url: "https://repo1.maven.org/maven2/com/ibm/db2/jcc/12.1.5.0/jcc-12.1.5.0.jar",
        filename: "jcc-12.1.5.0.jar",
        driver_class: "com.ibm.db2.jcc.DB2Driver",
        url_template: "jdbc:db2://{host}:{port}/{database}",
        default_port: 50000,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-h2",
        title: "H2",
        version: "2.4.240",
        url: "https://repo1.maven.org/maven2/com/h2database/h2/2.4.240/h2-2.4.240.jar",
        filename: "h2-2.4.240.jar",
        driver_class: "org.h2.Driver",
        url_template: "jdbc:h2:{file}",
        default_port: 0,
        notes: "jdbc:h2:tcp://host/db for server mode.",
    },
    DriverPackage {
        id: "jdbc-hsqldb",
        title: "HSQLDB",
        version: "2.7.4",
        url: "https://repo1.maven.org/maven2/org/hsqldb/hsqldb/2.7.4/hsqldb-2.7.4.jar",
        filename: "hsqldb-2.7.4.jar",
        driver_class: "org.hsqldb.jdbc.JDBCDriver",
        url_template: "jdbc:hsqldb:{file}",
        default_port: 9001,
        notes: "jdbc:hsqldb:hsql://host:port/db for server mode.",
    },
    DriverPackage {
        id: "jdbc-derby",
        title: "Apache Derby",
        version: "10.14.2.0",
        url: "https://repo1.maven.org/maven2/org/apache/derby/derby/10.14.2.0/derby-10.14.2.0.jar",
        filename: "derby-10.14.2.0.jar",
        driver_class: "org.apache.derby.jdbc.EmbeddedDriver",
        url_template: "jdbc:derby:{file};create=true",
        default_port: 1527,
        notes: "10.14 is the last self-contained jar (newer needs derbyshared/derbytools).",
    },
    DriverPackage {
        id: "jdbc-firebird",
        title: "Firebird (Jaybird)",
        version: "6.0.5",
        url: "https://repo1.maven.org/maven2/org/firebirdsql/jdbc/jaybird/6.0.5/jaybird-6.0.5.jar",
        filename: "jaybird-6.0.5.jar",
        driver_class: "org.firebirdsql.jdbc.FBDriver",
        url_template: "jdbc:firebird://{host}:{port}/{database}",
        default_port: 3050,
        notes: "Requires Java 17+.",
    },
    DriverPackage {
        id: "jdbc-trino",
        title: "Trino",
        version: "482",
        url: "https://repo1.maven.org/maven2/io/trino/trino-jdbc/482/trino-jdbc-482.jar",
        filename: "trino-jdbc-482.jar",
        driver_class: "io.trino.jdbc.TrinoDriver",
        url_template: "jdbc:trino://{host}:{port}/{catalog}",
        default_port: 8080,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-presto",
        title: "Presto",
        version: "0.298.1",
        url: "https://repo1.maven.org/maven2/com/facebook/presto/presto-jdbc/0.298.1/presto-jdbc-0.298.1.jar",
        filename: "presto-jdbc-0.298.1.jar",
        driver_class: "com.facebook.presto.jdbc.PrestoDriver",
        url_template: "jdbc:presto://{host}:{port}/{catalog}",
        default_port: 8080,
        notes: "PrestoDB (Meta lineage); use Trino for the fork.",
    },
    DriverPackage {
        id: "jdbc-vertica",
        title: "Vertica",
        version: "25.3.0-0",
        url: "https://repo1.maven.org/maven2/com/vertica/jdbc/vertica-jdbc/25.3.0-0/vertica-jdbc-25.3.0-0.jar",
        filename: "vertica-jdbc-25.3.0-0.jar",
        driver_class: "com.vertica.jdbc.Driver",
        url_template: "jdbc:vertica://{host}:{port}/{database}",
        default_port: 5433,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-exasol",
        title: "Exasol",
        version: "26.2.8",
        url: "https://repo1.maven.org/maven2/com/exasol/exasol-jdbc/26.2.8/exasol-jdbc-26.2.8.jar",
        filename: "exasol-jdbc-26.2.8.jar",
        driver_class: "com.exasol.jdbc.EXADriver",
        url_template: "jdbc:exa:{host}:{port}",
        default_port: 8563,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-teradata",
        title: "Teradata",
        version: "20.00.00.58",
        url: "https://repo1.maven.org/maven2/com/teradata/jdbc/terajdbc/20.00.00.58/terajdbc-20.00.00.58.jar",
        filename: "terajdbc-20.00.00.58.jar",
        driver_class: "com.teradata.jdbc.TeraDriver",
        url_template: "jdbc:teradata://{host}/DBS_PORT={port}",
        default_port: 1025,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-hana",
        title: "SAP HANA",
        version: "2.29.7",
        url: "https://repo1.maven.org/maven2/com/sap/cloud/db/jdbc/ngdbc/2.29.7/ngdbc-2.29.7.jar",
        filename: "ngdbc-2.29.7.jar",
        driver_class: "com.sap.db.jdbc.Driver",
        url_template: "jdbc:sap://{host}:{port}/?databaseName={database}",
        default_port: 30015,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-informix",
        title: "IBM Informix",
        version: "15.0.1.3",
        url: "https://repo1.maven.org/maven2/com/ibm/informix/jdbc/15.0.1.3/jdbc-15.0.1.3.jar",
        filename: "informix-jdbc-15.0.1.3.jar",
        driver_class: "com.informix.jdbc.IfxDriver",
        url_template: "jdbc:informix-sqli://{host}:{port}/{database}:INFORMIXSERVER={server}",
        default_port: 9088,
        notes: "",
    },
    DriverPackage {
        id: "jdbc-hive",
        title: "Apache Hive / Spark SQL",
        version: "4.2.0",
        url: "https://repo1.maven.org/maven2/org/apache/hive/hive-jdbc/4.2.0/hive-jdbc-4.2.0-standalone.jar",
        filename: "hive-jdbc-4.2.0-standalone.jar",
        driver_class: "org.apache.hive.jdbc.HiveDriver",
        url_template: "jdbc:hive2://{host}:{port}/{database}",
        default_port: 10000,
        notes: "Also connects to Spark Thrift Server; large standalone jar.",
    },
    DriverPackage {
        id: "jdbc-mongodb",
        title: "MongoDB (Atlas SQL)",
        version: "3.0.7",
        url: "https://repo1.maven.org/maven2/org/mongodb/mongodb-jdbc/3.0.7/mongodb-jdbc-3.0.7-all.jar",
        filename: "mongodb-jdbc-3.0.7-all.jar",
        driver_class: "com.mongodb.jdbc.MongoDriver",
        url_template: "jdbc:mongodb://{host}:{port}/{database}",
        default_port: 27017,
        notes: "Official MongoDB JDBC (Atlas SQL interface).",
    },
    DriverPackage {
        id: "jdbc-sybase",
        title: "Sybase ASE (jTDS)",
        version: "1.3.1",
        url: "https://repo1.maven.org/maven2/net/sourceforge/jtds/jtds/1.3.1/jtds-1.3.1.jar",
        filename: "jtds-1.3.1.jar",
        driver_class: "net.sourceforge.jtds.jdbc.Driver",
        url_template: "jdbc:jtds:sybase://{host}:{port}/{database}",
        default_port: 5000,
        notes: "Also speaks to legacy SQL Server (jdbc:jtds:sqlserver://…).",
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
