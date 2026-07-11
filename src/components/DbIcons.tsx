import { ConnectionProfile, JdbcParams } from "../api";

/**
 * Engine identity for icon/color purposes. JDBC connections are identified
 * by driver class so known engines get their own mark.
 */
export type EngineKind =
  | "sqlite"
  | "postgres"
  | "mysql"
  | "mariadb"
  | "duckdb"
  | "clickhouse"
  | "mssql"
  | "oracle"
  | "h2"
  | "trino"
  | "odbc"
  | "jdbc";

export function engineOf(profile: ConnectionProfile): EngineKind {
  switch (profile.driver) {
    case "sqlite":
      return "sqlite";
    case "postgres":
      return "postgres";
    case "mysql":
      return "mysql";
    case "odbc":
      return "odbc";
    case "jdbc": {
      const cls = ((profile.params as JdbcParams).driver_class ?? "").toLowerCase();
      if (cls.includes("postgresql")) return "postgres";
      if (cls.includes("mariadb")) return "mariadb";
      if (cls.includes("mysql")) return "mysql";
      if (cls.includes("sqlite")) return "sqlite";
      if (cls.includes("duckdb")) return "duckdb";
      if (cls.includes("clickhouse")) return "clickhouse";
      if (cls.includes("sqlserver")) return "mssql";
      if (cls.includes("oracle")) return "oracle";
      if (cls.includes("h2")) return "h2";
      if (cls.includes("trino")) return "trino";
      return "jdbc";
    }
  }
}

const ENGINE_COLORS: Record<EngineKind, string> = {
  postgres: "#699eca",
  mysql: "#e48e00",
  mariadb: "#c0765a",
  sqlite: "#67b8e3",
  duckdb: "#fff100",
  clickhouse: "#faff69",
  mssql: "#e8484f",
  oracle: "#f80000",
  h2: "#4a90d9",
  trino: "#dd00a1",
  odbc: "#9aa4b5",
  jdbc: "#f89820",
};

const ENGINE_MARKS: Record<EngineKind, string> = {
  postgres: "Pg",
  mysql: "My",
  mariadb: "Ma",
  sqlite: "Sq",
  duckdb: "Du",
  clickhouse: "Ch",
  mssql: "MS",
  oracle: "Or",
  h2: "H2",
  trino: "Tr",
  odbc: "OD",
  jdbc: "J",
};

/**
 * Database "cylinder" icon with an engine mark, tinted by connection state:
 * full engine color when 1+ sessions are open, muted gray otherwise.
 */
export function DbIcon({ engine, connected }: { engine: EngineKind; connected: boolean }) {
  const color = connected ? ENGINE_COLORS[engine] : "var(--icon-muted)";
  const textColor = connected ? "#10131a" : "var(--icon-muted-text)";
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" style={{ flexShrink: 0 }}>
      <ellipse cx="8" cy="3.4" rx="6.2" ry="2.6" fill={color} />
      <path d="M1.8 3.4 v9.2 c0 1.45 2.78 2.6 6.2 2.6 s6.2 -1.15 6.2 -2.6 V3.4" fill={color} />
      <ellipse cx="8" cy="3.4" rx="6.2" ry="2.6" fill="rgba(255,255,255,0.25)" />
      <text
        x="8"
        y="11.6"
        textAnchor="middle"
        fontSize="6.4"
        fontFamily="var(--sans)"
        fontWeight="700"
        fill={textColor}
      >
        {ENGINE_MARKS[engine]}
      </text>
      {connected && <circle cx="13.4" cy="13.2" r="2.4" fill="#39d353" stroke="#10131a" strokeWidth="0.8" />}
    </svg>
  );
}

/** Table icon: teal grid. */
export function TableIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" style={{ flexShrink: 0 }}>
      <rect x="1" y="1.5" width="12" height="11" rx="1.5" fill="none" stroke="#2aa79b" strokeWidth="1.4" />
      <line x1="1" y1="5.2" x2="13" y2="5.2" stroke="#2aa79b" strokeWidth="1.2" />
      <line x1="5.4" y1="5.2" x2="5.4" y2="12.5" stroke="#2aa79b" strokeWidth="1" />
      <line x1="9.4" y1="5.2" x2="9.4" y2="12.5" stroke="#2aa79b" strokeWidth="1" />
      <rect x="1" y="1.5" width="12" height="3.7" rx="1.5" fill="#2aa79b" opacity="0.35" />
    </svg>
  );
}

/** View icon: same table shape, purple, dashed to hint it is derived. */
export function ViewIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" style={{ flexShrink: 0 }}>
      <rect
        x="1"
        y="1.5"
        width="12"
        height="11"
        rx="1.5"
        fill="none"
        stroke="#a78bfa"
        strokeWidth="1.4"
        strokeDasharray="2.6 1.6"
      />
      <line x1="1" y1="5.2" x2="13" y2="5.2" stroke="#a78bfa" strokeWidth="1.2" />
      <line x1="5.4" y1="5.2" x2="5.4" y2="12.5" stroke="#a78bfa" strokeWidth="1" />
      <line x1="9.4" y1="5.2" x2="9.4" y2="12.5" stroke="#a78bfa" strokeWidth="1" />
      <rect x="1" y="1.5" width="12" height="3.7" rx="1.5" fill="#a78bfa" opacity="0.35" />
    </svg>
  );
}

/** Console icon for the tree's console entries. */
export function ConsoleIcon({ open }: { open: boolean }) {
  const c = open ? "var(--accent)" : "var(--text-dim)";
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" style={{ flexShrink: 0 }}>
      <rect x="1" y="2" width="12" height="10" rx="1.5" fill="none" stroke={c} strokeWidth="1.3" />
      <path d="M3.4 5.4 L5.8 7.2 L3.4 9" fill="none" stroke={c} strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
      <line x1="7" y1="9.4" x2="10.4" y2="9.4" stroke={c} strokeWidth="1.3" strokeLinecap="round" />
    </svg>
  );
}

/** +/- expander used across the schema tree. */
export function Expander({ expanded }: { expanded: boolean }) {
  return <span className="chev">{expanded ? "−" : "+"}</span>;
}
