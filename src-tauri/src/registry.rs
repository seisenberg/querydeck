use std::io::{Read, Write};

use serde::Serialize;
use sha2::{Digest, Sha256};
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
    /// Pinned lowercase SHA-256 of the jar at `url`, captured at development
    /// time from Maven Central. `download` verifies against this before the
    /// jar is promoted into place, so a tampered/substituted artifact is
    /// rejected instead of being loaded into the JVM.
    pub sha256: &'static str,
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
        sha256: "6e0e4cc2d8cae902084f8a2b18728b073a6fd9d1f87c9d8bff8f298c18185b93",
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
        sha256: "0353648eaa1c91e0f4020c959abf756bc866ffd583df22ae6b6f6e0cbd43eb44",
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
        sha256: "11e3bb5bbf8ef0e806ae4d6c5d5033fedf7262cc777f0190bde8a2f3c8e6bd8d",
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
        sha256: "dc320e4102884c135ccc30c3c6fc3fb190b750e1586a100e3aba3be783cf33a9",
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
        sha256: "6bfca0c795f78bab000de41e848e730011d5c3834592042460d5fe2bd68218fd",
        title: "DuckDB",
        version: "1.5.4.0",
        url: "https://repo1.maven.org/maven2/org/duckdb/duckdb_jdbc/1.5.4.0/duckdb_jdbc-1.5.4.0.jar",
        filename: "duckdb_jdbc-1.5.4.0.jar",
        driver_class: "org.duckdb.DuckDBDriver",
        url_template: "jdbc:duckdb:{file}",
        default_port: 0,
        notes: "Native (embedded) driver is built in; JDBC is optional for DuckDB.",
    },
    DriverPackage {
        id: "jdbc-clickhouse",
        sha256: "9dbe9fd70388b67429be7942f53b7984a4b71fd0f2f727f17339f20e27ecf564",
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
        sha256: "e36f5237c1267983e5b88dc2169f6b9d7e50eceec6dc1ca31018e3877e14af66",
        title: "Microsoft SQL Server (JDBC)",
        version: "13.4.0.jre11",
        url: "https://repo1.maven.org/maven2/com/microsoft/sqlserver/mssql-jdbc/13.4.0.jre11/mssql-jdbc-13.4.0.jre11.jar",
        filename: "mssql-jdbc-13.4.0.jre11.jar",
        driver_class: "com.microsoft.sqlserver.jdbc.SQLServerDriver",
        url_template: "jdbc:sqlserver://{host}:{port};databaseName={database};encrypt=true;trustServerCertificate=false",
        default_port: 1433,
        notes: "Native (TDS) driver is built in; also works for Azure SQL. TLS is verified by default; for a self-signed server cert set trustServerCertificate=true.",
    },
    DriverPackage {
        id: "jdbc-oracle",
        sha256: "dbc0ff940bc056d5d9b8f42c0946ded4ebbc08c25cecf6ec1e521b2c8216956b",
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
        sha256: "5d43d21df0e9b45b3a21c52889aea958177351ecd417bcad594ce7e9f5459082",
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
        sha256: "ec684f4096c622083b0efed2e49d20cd5246ed94b98da1e74ddaa8c573ba6e4d",
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
        id: "jdbc-databricks",
        sha256: "16c813a26eab7c461d71b29df3e3163ff0c601f61f9aa69a0e17d14c6c95bbe4",
        title: "Databricks",
        version: "3.4.1",
        url: "https://repo1.maven.org/maven2/com/databricks/databricks-jdbc/3.4.1/databricks-jdbc-3.4.1.jar",
        filename: "databricks-jdbc-3.4.1.jar",
        driver_class: "com.databricks.client.jdbc.Driver",
        url_template: "jdbc:databricks://{host}:443;httpPath={http-path};AuthMech=3;UID=token;PWD={personal-access-token}",
        default_port: 443,
        notes: "Official OSS driver; http-path from the SQL warehouse's connection details.",
    },
    DriverPackage {
        id: "jdbc-athena",
        sha256: "c185aa278ada9a12491fef14499a9d98af5673a0ff9a329d67e3cfea20ecf108",
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
        sha256: "04150111a29370247d162c3f05b8e938bf1314f19a63ad9a5302d88dd478e8e8",
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
        sha256: "29b70e427cc1c40cdc376283adbb0cc62853073797bb5fe5761f81fe73d57ce0",
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
        sha256: "5fab2bb4384ac06b762638c8fa2740c944b8d080e4796c0c6c2af8b90dd4e5ad",
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
        sha256: "2c40eb581e5221ab33c7c796979b49ce404e7e393357c58f7bcdb30a09efca72",
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
        sha256: "5aabfa8eac4136970c21edc5539b25227ee921a8d8251c8b4e6024f6adda0af4",
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
        sha256: "441d8958bef18055d6b1de6f1bc3dae74e6000090a8e7931fe34bd3bb01c815d",
        title: "Trino",
        version: "482",
        url: "https://repo1.maven.org/maven2/io/trino/trino-jdbc/482/trino-jdbc-482.jar",
        filename: "trino-jdbc-482.jar",
        driver_class: "io.trino.jdbc.TrinoDriver",
        url_template: "jdbc:trino://{host}:{port}/{catalog}",
        default_port: 8080,
        notes: "Native (HTTP) driver is built in; JDBC is optional for Trino.",
    },
    DriverPackage {
        id: "jdbc-presto",
        sha256: "eb22e8a06bc22974c94d3d08487fbf9e7b46360b391b3e2e554dc69421bccff2",
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
        sha256: "f43c4ed61a9639d95214e00c888fd21c4ea283d60ca2ce06c6855a774ff5e583",
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
        sha256: "e8982813756ec2715a2bd2089169d08f1138031e7c26de02e62558df1883a459",
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
        sha256: "4272190ac97083bbcc31117cc940984c05dc2fb0044fb74cdf660952b4760b60",
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
        sha256: "d6e17ea4e91e1749e68fa383f0bf1d431c619b1408ab85d522f3617979b12445",
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
        sha256: "6e1e9fea09385e8d99abdc5ba969c4c36e747675945baf525b855882e078848d",
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
        sha256: "c89c6224df9d465ddf96060bc3e289522f1f6e64b01a28e01f94413edeb3d43a",
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
        sha256: "2e51d2d20df07dca35b11c9c87f542fb7382b11a61f1fa7744058e70c57e458e",
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
        sha256: "aac05ebf5504c91b29420129b02dd878a86c52f8fa6eccf9235e0bfd7a60bef1",
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
    let pkg = find_package(id)?;
    fetch_verified(app, id, pkg.url, pkg.filename, pkg.sha256)
}

/// Download a specific (possibly newer-than-pinned) version of a package.
///
/// The pinned version verifies against the compiled-in SHA-256. Other
/// versions verify against the `.sha256` checksum Maven Central publishes
/// beside every artifact — same origin as the jar, so this is a corruption /
/// truncation check rather than a defense against a compromised repository,
/// which is the intended scope for user-initiated upgrades.
pub fn download_version(app: &tauri::AppHandle, id: &str, version: &str) -> AppResult<String> {
    let pkg = find_package(id)?;
    if version == pkg.version {
        return download(app, id);
    }
    let parts = maven_parts(pkg)
        .ok_or_else(|| AppError::config(format!("cannot derive Maven coordinates for {id}")))?;
    if !version_eligible(pkg.version, version) {
        return Err(AppError::config(format!(
            "refusing unexpected version {version} for {id}"
        )));
    }
    let url = parts.jar_url(version);
    let sha = http_client()?
        .get(format!("{url}.sha256"))
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.text())
        .map_err(|e| AppError::io(format!("checksum fetch failed: {e}")))?;
    // Checksum files are "<hex>" or "<hex>  <filename>".
    let sha = sha.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::io(format!("no usable checksum published for {url}")));
    }
    fetch_verified(app, id, &url, &parts.jar_filename(version), &sha)
}

fn find_package(id: &str) -> AppResult<&'static DriverPackage> {
    PACKAGES
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::config(format!("unknown driver package {id}")))
}

fn http_client() -> AppResult<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::io(format!("http client: {e}")))
}

fn fetch_verified(
    app: &tauri::AppHandle,
    id: &str,
    url: &str,
    filename: &str,
    expected_sha: &str,
) -> AppResult<String> {
    let dir = AppState::drivers_dir();
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(filename);
    let tmp = dir.join(format!("{filename}.part"));

    let mut resp = reqwest::blocking::get(url)
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
    let bytes = std::fs::read(&tmp)?;
    if bytes.len() < 4 || &bytes[..2] != b"PK" {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::io("downloaded file is not a jar (bad content)"));
    }

    // Integrity: the jar is loaded into a JVM, so verify it byte-for-byte
    // against the expected SHA-256 (compiled-in pin for the catalog version,
    // Maven's published checksum for user-requested upgrades) before
    // promoting it into place. A CDN glitch, truncation, or substituted
    // artifact fails here instead of running as trusted code.
    let digest = hex::encode(Sha256::digest(&bytes));
    if !digest.eq_ignore_ascii_case(expected_sha) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::io(format!(
            "checksum mismatch for {filename}: expected {expected_sha}, got {digest} — refusing to install"
        )));
    }

    std::fs::rename(&tmp, &dest)?;
    let _ = app.emit(
        "driver:progress",
        DownloadProgress { id: id.to_string(), done, total: done },
    );
    Ok(dest.to_string_lossy().into_owned())
}

// ------------------------------------------------------------ freshness --
//
// Staleness is advisory only: connections keep whatever jar they point at
// (some servers need old drivers), and upgrading is a per-connection button
// in the editor. The catalog pins stay the offline baseline; Maven Central's
// maven-metadata.xml supplies "newest available" at runtime, cached on disk
// so the check is at most one network sweep per day.

/// Maven coordinates derived from a package's pinned repo1 URL, which always
/// has the shape `<artifact_dir>/<version>/<artifact>-<version><suffix>`.
/// The on-disk filename may use a different prefix than the artifact (e.g.
/// Informix's artifact is just "jdbc", stored as "informix-jdbc-…"), so local
/// and remote naming are derived separately.
struct MavenParts {
    artifact_dir: String,
    artifact: String,
    /// Filename tail after the version: ".jar", or classifier forms like
    /// "-all.jar" (ClickHouse) and "-standalone.jar" (Hive).
    suffix: String,
    /// Local drivers-dir filename prefix up to the version.
    local_prefix: String,
}

impl MavenParts {
    fn jar_filename(&self, version: &str) -> String {
        format!("{}{}{}", self.local_prefix, version, self.suffix)
    }
    fn jar_url(&self, version: &str) -> String {
        format!(
            "{}/{}/{}-{}{}",
            self.artifact_dir, version, self.artifact, version, self.suffix
        )
    }
}

fn maven_parts(pkg: &DriverPackage) -> Option<MavenParts> {
    let (rest, file) = pkg.url.rsplit_once('/')?;
    let (artifact_dir, version) = rest.rsplit_once('/')?;
    if version != pkg.version {
        return None;
    }
    let artifact = artifact_dir.rsplit_once('/')?.1;
    let suffix = file.strip_prefix(&format!("{artifact}-{version}"))?;
    let vi = pkg.filename.find(pkg.version)?;
    if &pkg.filename[vi + pkg.version.len()..] != suffix {
        return None;
    }
    Some(MavenParts {
        artifact_dir: artifact_dir.to_string(),
        artifact: artifact.to_string(),
        suffix: suffix.to_string(),
        local_prefix: pkg.filename[..vi].to_string(),
    })
}

/// Split off a ".jreNN" tail (MSSQL publishes the jre8 and jre11 lines as
/// separate versions); comparisons stay within the pinned family.
fn split_jre(v: &str) -> (&str, &str) {
    match v.find(".jre") {
        Some(i) => v.split_at(i),
        None => (v, ""),
    }
}

fn is_plain_numeric(v: &str) -> bool {
    !v.is_empty()
        && v.split(['.', '-'])
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

/// A candidate qualifies when it is in the same ".jre" family as the pinned
/// version and is otherwise plain numeric — which wholesale-rejects
/// prerelease tags (rc/beta/alpha/M1/SNAPSHOT/…) without a word list.
fn version_eligible(pinned: &str, candidate: &str) -> bool {
    let (_, pin_fam) = split_jre(pinned);
    let (cand_base, cand_fam) = split_jre(candidate);
    cand_fam == pin_fam && is_plain_numeric(cand_base)
}

/// Numeric segment-wise order ("42.7.2" < "42.10.0"); absent segments are 0.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let segs = |v: &str| -> Vec<u64> {
        split_jre(v)
            .0
            .split(['.', '-'])
            .map(|s| s.parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (segs(a), segs(b));
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| {
            a.get(i)
                .copied()
                .unwrap_or(0)
                .cmp(&b.get(i).copied().unwrap_or(0))
        })
        .find(|o| *o != std::cmp::Ordering::Equal)
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// All published versions of an artifact, scraped from maven-metadata.xml.
/// A ~40-line string scan beats an XML dependency for this fixed format.
fn fetch_versions(parts: &MavenParts) -> AppResult<Vec<String>> {
    let xml = http_client()?
        .get(format!("{}/maven-metadata.xml", parts.artifact_dir))
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.text())
        .map_err(|e| AppError::io(format!("metadata fetch failed: {e}")))?;
    let mut out = Vec::new();
    let mut rest = xml.as_str();
    while let Some(i) = rest.find("<version>") {
        rest = &rest[i + "<version>".len()..];
        let Some(j) = rest.find("</version>") else { break };
        out.push(rest[..j].trim().to_string());
        rest = &rest[j..];
    }
    Ok(out)
}

/// What the connection editor needs to flag a stale jar and offer an upgrade.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DriverLatest {
    pub id: String,
    /// Newest eligible release on Maven Central (never older than the pin;
    /// falls back to the pin when offline or the metadata fetch fails).
    pub latest: String,
    /// Whether the jar for `latest` is already in the drivers dir.
    pub downloaded: bool,
    pub path: Option<String>,
}

#[derive(serde::Deserialize, Serialize, Default)]
struct LatestCache {
    checked_at: u64,
    versions: std::collections::HashMap<String, String>,
}

const LATEST_TTL_SECS: u64 = 24 * 60 * 60;

fn latest_cache_path() -> std::path::PathBuf {
    AppState::drivers_dir().join("latest-versions.json")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn check_latest(force: bool) -> Vec<DriverLatest> {
    let mut cache: LatestCache = std::fs::read(latest_cache_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let fresh = !cache.versions.is_empty()
        && now_secs().saturating_sub(cache.checked_at) < LATEST_TTL_SECS;
    if force || !fresh {
        // One thread per package; a failed or offline fetch simply leaves
        // that package at its pinned version (no staleness signal, no error).
        let results: Vec<(&str, Option<String>)> = std::thread::scope(|s| {
            let handles: Vec<_> = PACKAGES
                .iter()
                .map(|p| {
                    s.spawn(move || {
                        let latest = maven_parts(p).and_then(|parts| {
                            fetch_versions(&parts)
                                .ok()?
                                .into_iter()
                                .filter(|v| version_eligible(p.version, v))
                                .chain(std::iter::once(p.version.to_string()))
                                .max_by(|a, b| version_cmp(a, b))
                        });
                        (p.id, latest)
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|h| h.join().ok())
                .collect()
        });
        cache.versions = results
            .into_iter()
            .filter_map(|(id, v)| Some((id.to_string(), v?)))
            .collect();
        cache.checked_at = now_secs();
        let _ = std::fs::create_dir_all(AppState::drivers_dir());
        if let Ok(json) = serde_json::to_vec_pretty(&cache) {
            let _ = std::fs::write(latest_cache_path(), json);
        }
    }

    let dir = AppState::drivers_dir();
    PACKAGES
        .iter()
        .map(|p| {
            let latest = cache
                .versions
                .get(p.id)
                .cloned()
                .unwrap_or_else(|| p.version.to_string());
            let filename = maven_parts(p)
                .map(|parts| parts.jar_filename(&latest))
                .unwrap_or_else(|| p.filename.to_string());
            let path = dir.join(&filename);
            let downloaded = path.exists();
            DriverLatest {
                id: p.id.to_string(),
                latest,
                downloaded,
                path: downloaded.then(|| path.to_string_lossy().into_owned()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{maven_parts, version_cmp, version_eligible, PACKAGES};
    use std::cmp::Ordering;

    /// Every package URL must decompose into Maven coordinates that
    /// round-trip to the pinned filename and URL — the upgrade path builds
    /// newer-version URLs from exactly this derivation.
    #[test]
    fn maven_parts_roundtrip_for_all_packages() {
        for p in PACKAGES {
            let parts = maven_parts(p).unwrap_or_else(|| panic!("{}: underivable", p.id));
            assert_eq!(parts.jar_filename(p.version), p.filename, "{}", p.id);
            assert_eq!(parts.jar_url(p.version), p.url, "{}", p.id);
        }
    }

    #[test]
    fn version_ordering_and_eligibility() {
        assert_eq!(version_cmp("42.7.2", "42.10.0"), Ordering::Less);
        assert_eq!(version_cmp("482", "479"), Ordering::Greater);
        assert_eq!(version_cmp("25.3.0-0", "25.3.0-1"), Ordering::Less);
        assert_eq!(version_cmp("13.4.0.jre11", "13.4.1.jre11"), Ordering::Less);

        // Prereleases and cross-family versions never qualify.
        assert!(version_eligible("42.7.13", "42.8.0"));
        assert!(!version_eligible("42.7.13", "42.8.0-rc1"));
        assert!(!version_eligible("42.7.13", "42.8.0.beta2"));
        assert!(version_eligible("13.4.0.jre11", "13.6.0.jre11"));
        assert!(!version_eligible("13.4.0.jre11", "13.6.0.jre8"));
        assert!(!version_eligible("13.4.0.jre11", "13.6.0"));
        assert!(!version_eligible("1.3.1", "1.3.2.jre11"));
    }

    /// Guards the pinned integrity data: every package must carry a
    /// well-formed lowercase 64-char SHA-256, and ids must be unique. A typo or
    /// truncated hash would make `download` reject a legitimate jar, so catch
    /// it at test time rather than in the field.
    #[test]
    fn pinned_sha256_is_wellformed_and_ids_unique() {
        let mut seen = std::collections::HashSet::new();
        for p in PACKAGES {
            assert!(seen.insert(p.id), "duplicate package id {}", p.id);
            assert_eq!(p.sha256.len(), 64, "{}: sha256 must be 64 hex chars", p.id);
            assert!(
                p.sha256
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "{}: sha256 must be lowercase hex",
                p.id
            );
        }
    }
}
