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
  const profiles: any[] = [
    {
      id: "demo-sqlite",
      name: "demo.db",
      driver: "sqlite",
      params: { path: "/tmp/demo.db" },
    },
    {
      id: "demo-ch",
      name: "ClickHouse play",
      driver: "clickhouse",
      params: {
        host: "play.clickhouse.com",
        port: 443,
        user: "explorer",
        password: "",
        database: "",
        use_https: true,
        accept_invalid_certs: false,
      },
    },
    {
      id: "demo-pg",
      name: "orders@prod",
      driver: "postgres",
      params: {
        host: "localhost",
        port: 5432,
        user: "postgres",
        password: "__keychain__",
        database: "orders",
        ssl_mode: "prefer",
        accept_invalid_certs: false,
      },
    },
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

  function fakeRows(n: number): unknown[][] {
    const names = ["Ada", "Grace", "Alan", "Edsger", "Barbara", "Donald"];
    return Array.from({ length: n }, (_, i) => [
      i + 1,
      names[i % names.length],
      `${names[i % names.length].toLowerCase()}@example.com`,
      (Math.round(((i * 37) % 900) + 100) / 10).toFixed(2),
    ]);
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

    const rowCount = 24;
    const resultId = nextResult++;
    results.set(resultId, fakeRows(rowCount));
    return {
      statements: [
        {
          sql: String(args.sql).slice(0, 120),
          kind: "rows",
          resultId,
          columns: [
            { name: "id", typeName: "bigint" },
            { name: "name", typeName: "text" },
            { name: "email", typeName: "text" },
            { name: "total", typeName: "numeric" },
          ],
          rowCount,
          truncated: false,
          affected: null,
          elapsedMs: Date.now() - started,
        },
      ],
      error: null,
      totalElapsedMs: Date.now() - started,
    };
  }

  const store: Record<string, string> = {};

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
    sessions_count: () => ({ "demo-sqlite": 1 }),
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
    session_state_load: () => store["session"] ?? null,
    config_save: (a) => {
      store["config"] = a.json;
    },
    config_load: () => store["config"] ?? null,
    env_status: () => ({
      odbcAvailable: true,
      odbcDetail: "mock",
      jvmAvailable: true,
      jvmDetail: "mock",
    }),
    drivers_registry: () => [],
    driver_download: () => "",
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
