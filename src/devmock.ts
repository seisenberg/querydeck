// Browser-preview mock of the Tauri IPC layer.
//
// Loaded (side-effect import) before the store initializes. When the page
// runs inside the real Tauri app, __TAURI_INTERNALS__ exists and this file
// does nothing. In a plain browser (vite dev server opened directly) it
// installs an in-memory backend so the full UI is drivable for development
// and visual testing — sample connections, catalog, saved consoles, fake
// query execution and activity events.

/* eslint-disable @typescript-eslint/no-explicit-any */

interface Win extends Window {
  __TAURI_INTERNALS__?: any;
  __QD_MOCK__?: boolean;
}

const w = window as Win;

if (!w.__TAURI_INTERNALS__) {
  w.__QD_MOCK__ = true;

  // ---- tiny event bus (backs @tauri-apps/api/event) ----
  let nextCb = 1;
  const callbacks = new Map<number, (payload: unknown) => void>();
  const listeners = new Map<string, Set<number>>();

  function emit(event: string, payload?: unknown) {
    for (const id of listeners.get(event) ?? []) {
      callbacks.get(id)?.({ event, id, payload });
    }
  }

  // ---- sample data ----
  const server = (host: string, port: number, user: string, database: string) => ({
    host,
    port,
    user,
    password: "__keychain__",
    database,
    ssl_mode: "prefer",
    accept_invalid_certs: false,
  });
  const jdbc = (
    cls: string,
    url: string,
    // Default is deliberately unmatchable by the staleness detector; the
    // snowflake profile overrides with an old catalog-shaped jar so the
    // "newer driver available" badge is exercisable in the preview.
    jar = "/Users/demo/Library/Application Support/QueryDeck/drivers/driver.jar",
  ) => ({
    url,
    driver_class: cls,
    jars: [jar],
    user: "svc_analytics",
    password: "__keychain__",
  });
  const profiles: any[] = [
    { id: "demo-pg", name: "orders@prod", driver: "postgres", params: server("db.prod.internal", 5432, "app", "orders") },
    { id: "demo-redshift", name: "warehouse", driver: "redshift", params: server("analytics.xyz.us-east-1.redshift.amazonaws.com", 5439, "awsuser", "dev") },
    { id: "demo-snowflake", name: "SNOWFLAKE_ANALYTICS", driver: "jdbc", params: jdbc("net.snowflake.client.jdbc.SnowflakeDriver", "jdbc:snowflake://acme-prod.snowflakecomputing.com/?db=ANALYTICS", "/Users/demo/Library/Application Support/QueryDeck/drivers/jdbc-snowflake-4.1.0.jar") },
    { id: "demo-ch", name: "events@clickhouse", driver: "clickhouse", params: { host: "ch.internal", port: 8123, user: "default", password: "", database: "events", use_https: false, accept_invalid_certs: false } },
    { id: "demo-databricks", name: "lakehouse", driver: "jdbc", params: jdbc("com.databricks.client.jdbc.Driver", "jdbc:databricks://dbc-123.cloud.databricks.com:443") },
    { id: "demo-mssql", name: "crm", driver: "mssql", params: { ...server("sql01.corp", 1433, "sa", "CRM"), encrypt: true } },
    { id: "demo-mysql", name: "shop", driver: "mysql", params: { ...server("mysql.shop.internal", 3306, "root", "shop"), use_ssl: true } },
    { id: "demo-mariadb", name: "wiki", driver: "mariadb", params: server("maria.internal", 3306, "wiki", "wiki") },
    { id: "demo-duckdb", name: "local.duckdb", driver: "duckdb", params: { path: "/Users/demo/data/local.duckdb" } },
    { id: "demo-trino", name: "federation", driver: "trino", params: { host: "trino.internal", port: 8080, user: "trino", password: "", catalog: "iceberg", schema: "", use_https: false, accept_invalid_certs: false } },
    { id: "demo-sqlite", name: "demo.db", driver: "sqlite", params: { path: "/tmp/demo.db" } },
  ];

  const catalog = {
    schemas: [
      {
        name: "public",
        tables: [
          {
            name: "customers",
            kind: "table",
            pk: ["id"],
            columns: [
              { name: "id", dataType: "bigint", nullable: false },
              { name: "name", dataType: "text", nullable: false },
              { name: "email", dataType: "text", nullable: true },
              { name: "created_at", dataType: "timestamptz", nullable: false },
            ],
          },
          {
            name: "orders",
            kind: "table",
            pk: ["id"],
            columns: [
              { name: "id", dataType: "bigint", nullable: false },
              { name: "customer_id", dataType: "bigint", nullable: false },
              { name: "total", dataType: "numeric(12,2)", nullable: false },
              { name: "status", dataType: "text", nullable: false },
            ],
          },
          {
            name: "order_totals",
            kind: "view",
            pk: [],
            columns: [
              { name: "customer_id", dataType: "bigint", nullable: true },
              { name: "total", dataType: "numeric", nullable: true },
            ],
          },
        ],
      },
      {
        name: "analytics",
        tables: [
          {
            name: "daily_revenue",
            kind: "materialized view",
            pk: [],
            columns: [
              { name: "day", dataType: "date", nullable: true },
              { name: "revenue", dataType: "numeric", nullable: true },
            ],
          },
        ],
      },
    ],
  };

  const objects = [
    { schema: "public", name: "fn_total_for", kind: "function", detail: "customer_id bigint" },
    { schema: "public", name: "fn_email_valid", kind: "function", detail: "addr text" },
    { schema: "public", name: "sp_reprice", kind: "procedure", detail: "" },
    { schema: "public", name: "orders_id_seq", kind: "sequence" },
    { schema: "public", name: "trg_orders_audit", kind: "trigger", detail: "orders" },
    { schema: "analytics", name: "fn_rollup", kind: "function", detail: "" },
    { name: "app_rw", kind: "role", detail: "login role" },
    { name: "app_ro", kind: "role", detail: "group role" },
  ];

  const savedConsoles: any[] = [
    {
      id: "saved-1",
      title: "console 1",
      profileId: "demo-sqlite",
      database: null,
      sql: "SELECT name, email FROM customers WHERE created_at > date('now', '-7 day');",
      log: [],
      updatedAt: "2026-07-10 14:22:03",
      closed: true,
    },
    {
      id: "saved-2",
      title: "revenue check",
      profileId: "demo-pg",
      database: null,
      sql: "SELECT * FROM analytics.daily_revenue ORDER BY day DESC LIMIT 30;",
      log: [],
      updatedAt: "2026-07-09 09:10:41",
      closed: true,
    },
  ];

  const history = [
    "SELECT id, name, email FROM customers ORDER BY created_at DESC LIMIT 50",
    "SELECT status, count(*) FROM orders GROUP BY status",
    "SELECT c.name, sum(o.total) AS lifetime FROM customers c JOIN orders o ON o.customer_id = c.id GROUP BY c.name ORDER BY lifetime DESC",
    "UPDATE orders SET status = 'shipped' WHERE id = 4211",
  ].map((sql, i) => ({
    ts: `2026-07-1${i} 12:0${i}:00`,
    profileId: "demo-sqlite",
    profileName: "demo.db",
    sql,
    kind: "rows",
    rowCount: 10 * (i + 1),
    elapsedMs: 12 + i,
  }));

  const activity: any[] = [];
  let nextActivity = 1;
  let nextResult = 1;
  const results = new Map<number, unknown[][]>();

  /** Monthly revenue per customer segment — matches the seeded demo query. */
  function fakeRows(): unknown[][] {
    const segments = ["enterprise", "smb", "consumer"];
    const base: Record<string, number> = { enterprise: 184_000, smb: 61_000, consumer: 23_500 };
    const rows: unknown[][] = [];
    for (let m = 0; m < 12; m++) {
      const month = new Date(Date.UTC(2026, 6 - m, 1)).toISOString().slice(0, 10);
      for (const seg of segments) {
        const wobble = 1 + 0.18 * Math.sin(m * 1.7 + seg.length) - m * 0.012;
        const revenue = Math.round(base[seg] * wobble * 100) / 100;
        const buyers = Math.max(4, Math.round(revenue / (seg === "enterprise" ? 9200 : seg === "smb" ? 1150 : 61)));
        rows.push([month, seg, revenue, buyers, Math.round((revenue / buyers) * 100) / 100]);
      }
    }
    return rows;
  }

  async function runQuery(args: any) {
    const started = Date.now();
    const id = nextActivity++;
    activity.push({
      id,
      kind: "query",
      label: String(args.sql).slice(0, 80),
      profileId: args.profileId,
      startedMs: started,
    });
    emit("activity:changed");
    await new Promise((r) => setTimeout(r, 700));
    activity.splice(activity.findIndex((a) => a.id === id), 1);
    emit("activity:changed");

    const rows = fakeRows();
    const resultId = nextResult++;
    results.set(resultId, rows);
    return {
      statements: [
        {
          sql: String(args.sql).replace(/\s+/g, " ").slice(0, 120),
          kind: "rows",
          resultId,
          columns: [
            { name: "month", typeName: "date" },
            { name: "segment", typeName: "text" },
            { name: "revenue", typeName: "numeric" },
            { name: "buyers", typeName: "bigint" },
            { name: "revenue_per_buyer", typeName: "numeric" },
          ],
          rowCount: rows.length,
          truncated: false,
          affected: null,
          elapsedMs: 143,
        },
      ],
      error: null,
      totalElapsedMs: 143,
    };
  }

  /** Session seeded with a few consoles so a fresh preview looks lived-in. */
  const DEMO_SQL = `-- monthly revenue and repeat purchase rate by segment
WITH monthly AS (
  SELECT
    date_trunc('month', o.created_at) AS month,
    c.segment,
    sum(o.total) AS revenue,
    count(DISTINCT o.customer_id) AS buyers
  FROM orders o
  JOIN customers c ON c.id = o.customer_id
  WHERE o.status NOT IN ('cancelled', 'refunded')
  GROUP BY 1, 2
)
SELECT
  month,
  segment,
  revenue,
  buyers,
  round(revenue / buyers, 2) AS revenue_per_buyer
FROM monthly
ORDER BY month DESC, revenue DESC;`;

  const seededSession = JSON.stringify({
    tabs: [
      { id: "tab-rev", title: "revenue by segment", profileId: "demo-pg", database: null, sql: DEMO_SQL, maxRows: 1000 },
      { id: "tab-events", title: "console 1", profileId: "demo-ch", database: null, sql: "SELECT event, count() AS n FROM events.pageviews WHERE ts > now() - INTERVAL 1 DAY GROUP BY event ORDER BY n DESC LIMIT 20;", maxRows: 1000 },
      { id: "tab-sf", title: "usage rollup", profileId: "demo-snowflake", database: null, sql: "SELECT account_id, sum(credits_used) FROM usage.daily GROUP BY 1 ORDER BY 2 DESC;", maxRows: 1000 },
    ],
    activeTabId: "tab-rev",
    ui: { sidebarWidth: 280, sidebarCollapsed: false, editorFrac: 0.5 },
  });

  const store: Record<string, string> = {};

  function pkg(
    id: string,
    title: string,
    version: string,
    driverClass: string,
    urlTemplate: string,
    defaultPort: number,
    notes: string,
    downloaded: boolean,
  ) {
    return {
      id,
      title,
      version,
      url: "",
      filename: `${id}-${version}.jar`,
      driverClass,
      urlTemplate,
      defaultPort,
      notes,
      downloaded,
      path: downloaded
        ? `/Users/demo/Library/Application Support/QueryDeck/drivers/${id}-${version}.jar`
        : null,
    };
  }

  const handlers: Record<string, (args: any) => unknown | Promise<unknown>> = {
    profiles_list: () => profiles,
    profile_save: (a) => {
      const i = profiles.findIndex((p) => p.id === a.profile.id);
      if (i === -1) profiles.push(a.profile);
      else profiles[i] = a.profile;
    },
    profile_delete: (a) => {
      const i = profiles.findIndex((p) => p.id === a.profileId);
      if (i !== -1) profiles.splice(i, 1);
    },
    profiles_reorder: (a) => {
      profiles.sort(
        (x, y) => a.ids.indexOf(x.id) - a.ids.indexOf(y.id),
      );
    },
    connection_test: async () => {
      await new Promise((r) => setTimeout(r, 400));
      return "Connection OK (mock)";
    },
    query_run: runQuery,
    query_cancel: () => undefined,
    page_fetch: (a) => ({
      rows: (results.get(a.resultId) ?? []).slice(a.offset, a.offset + a.limit),
    }),
    results_free: () => undefined,
    session_close: () => undefined,
    catalog_get: async (a) => {
      if (a.refresh) await new Promise((r) => setTimeout(r, 300));
      return catalog;
    },
    objects_list: () => objects,
    object_ddl: (a) => `-- mock DDL\nCREATE ${a.kind.toUpperCase()} ${a.name} …;`,
    ddl_get: (a) => ({ ddl: `-- mock\nCREATE TABLE ${a.schema}.${a.table} (…);`, native: false }),
    sessions_count: () => ({
      "demo-pg": 2,
      "demo-snowflake": 1,
      "demo-ch": 1,
      "demo-redshift": 1,
      "demo-databricks": 1,
      "demo-sqlite": 1,
      "demo-mssql": 1,
      "demo-duckdb": 1,
    }),
    databases_list: () => ["orders", "postgres", "analytics"],
    profile_disconnect: () => undefined,
    history_list: () => history,
    history_clear: () => undefined,
    console_save: (a) => {
      const i = savedConsoles.findIndex((c) => c.id === a.console.id);
      if (i === -1) savedConsoles.push(a.console);
      else savedConsoles[i] = a.console;
    },
    console_list: () => savedConsoles,
    console_delete: (a) => {
      const i = savedConsoles.findIndex((c) => c.id === a.consoleId);
      if (i !== -1) savedConsoles.splice(i, 1);
    },
    session_state_save: (a) => {
      store["session"] = a.json;
    },
    session_state_load: () => store["session"] ?? seededSession,
    config_save: (a) => {
      store["config"] = a.json;
    },
    config_load: () => store["config"] ?? null,
    env_status: () => ({
      odbcAvailable: true,
      odbcDetail: "unixODBC 2.3.12 loaded",
      jvmAvailable: true,
      jvmDetail: "OpenJDK 21 (Temurin) via JAVA_HOME",
    }),
    drivers_registry: () => [
      pkg("jdbc-snowflake", "Snowflake", "4.3.1", "net.snowflake.client.jdbc.SnowflakeDriver", "jdbc:snowflake://{account}.snowflakecomputing.com/?db={database}", 443, "Account is the org-account identifier.", true),
      pkg("jdbc-databricks", "Databricks", "3.4.1", "com.databricks.client.jdbc.Driver", "jdbc:databricks://{host}:443;httpPath={http-path}", 443, "Official OSS driver.", true),
      pkg("jdbc-duckdb", "DuckDB", "1.5.4.0", "org.duckdb.DuckDBDriver", "jdbc:duckdb:{file}", 0, "Empty path = in-memory.", true),
      pkg("jdbc-oracle", "Oracle Database", "23.26.2.0.0", "oracle.jdbc.OracleDriver", "jdbc:oracle:thin:@//{host}:{port}/{service}", 1521, "", false),
      pkg("jdbc-db2", "IBM Db2", "12.1.5.0", "com.ibm.db2.jcc.DB2Driver", "jdbc:db2://{host}:{port}/{database}", 50000, "", false),
      pkg("jdbc-teradata", "Teradata", "20.00.00.58", "com.teradata.jdbc.TeraDriver", "jdbc:teradata://{host}/DBS_PORT={port}", 1025, "", false),
      pkg("jdbc-vertica", "Vertica", "25.3.0-0", "com.vertica.jdbc.Driver", "jdbc:vertica://{host}:{port}/{database}", 5433, "", false),
      pkg("jdbc-hana", "SAP HANA", "2.29.7", "com.sap.db.jdbc.Driver", "jdbc:sap://{host}:{port}/", 30015, "", false),
      pkg("jdbc-trino", "Trino", "482", "io.trino.jdbc.TrinoDriver", "jdbc:trino://{host}:{port}/{catalog}", 8080, "", false),
      pkg("jdbc-hive", "Apache Hive / Spark SQL", "4.2.0", "org.apache.hive.jdbc.HiveDriver", "jdbc:hive2://{host}:{port}/{database}", 10000, "Also Spark Thrift Server.", false),
      pkg("jdbc-exasol", "Exasol", "26.2.8", "com.exasol.jdbc.EXADriver", "jdbc:exa:{host}:{port}", 8563, "", false),
      pkg("jdbc-athena", "Amazon Athena", "2026.24.1", "com.amazonaws.athena.jdbc.AthenaDriver", "jdbc:athena://Region={region};", 443, "AWS credential chain.", false),
    ],
    driver_download: () => "",
    // Snowflake reports a newer release than the mock profile's 4.1.0 jar so
    // the stale-jar badge and Update button are exercisable in the preview.
    drivers_check_latest: () => [
      { id: "jdbc-snowflake", latest: "4.6.2", downloaded: false, path: null },
    ],
    driver_download_version: (a) =>
      `/Users/demo/Library/Application Support/QueryDeck/drivers/${a.id}-${a.version}.jar`,
    drivers_dir: () => "/tmp",
    odbc_inventory: () => ({ drivers: [], dsns: [] }),
    tunnels_list: () => [],
    tunnels_status: () => [],
    // Fresh array each call — the real IPC layer deserializes anew, and the
    // store's reference-equality checks depend on that.
    activity_list: () => [...activity],
    background_kill: () => undefined,
  };

  w.__TAURI_INTERNALS__ = {
    transformCallback(cb: (payload: unknown) => void): number {
      const id = nextCb++;
      callbacks.set(id, cb);
      return id;
    },
    async invoke(cmd: string, args: any = {}): Promise<unknown> {
      if (cmd === "plugin:event|listen") {
        const set = listeners.get(args.event) ?? new Set<number>();
        set.add(args.handler);
        listeners.set(args.event, set);
        return args.handler;
      }
      if (cmd === "plugin:event|unlisten") {
        listeners.get(args.event)?.delete(args.eventId);
        return;
      }
      const h = handlers[cmd];
      if (!h) throw { kind: "internal", message: `mock: unhandled command ${cmd}` };
      return h(args);
    },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };

  // Hook for driving scenarios from the devtools console / test harness.
  (w as any).__QD_EMIT__ = emit;
  (w as any).__QD_ACTIVITY__ = activity;
  console.info("[querydeck] running with mock Tauri backend (browser preview)");
}

export {};
