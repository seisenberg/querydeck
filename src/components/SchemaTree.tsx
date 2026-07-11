import { useEffect, useState } from "react";
import { ConnectionProfile, ConsoleFile, TableInfo } from "../api";
import { useStore } from "../store";
import { ConsoleIcon, DbIcon, Expander, TableIcon, ViewIcon, engineOf } from "./DbIcons";

interface CtxMenu {
  x: number;
  y: number;
  items: { label: string; danger?: boolean; action: () => void }[];
}

export default function SchemaTree() {
  const profiles = useStore((s) => s.profiles);
  const openConnectionModal = useStore((s) => s.openConnectionModal);
  const [ctx, setCtx] = useState<CtxMenu | null>(null);

  useEffect(() => {
    if (!ctx) return;
    const close = () => setCtx(null);
    window.addEventListener("click", close);
    window.addEventListener("contextmenu", close);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("contextmenu", close);
    };
  }, [ctx]);

  return (
    <>
      <div className="sidebar-header">
        <span>Connections</span>
        <span className="actions">
          <button className="small ghost" title="New connection" onClick={() => openConnectionModal()}>
            +
          </button>
        </span>
      </div>
      <div className="tree">
        {profiles.length === 0 && (
          <div className="tree-empty">
            No connections yet.
            <br />
            <button className="primary" style={{ marginTop: 8 }} onClick={() => openConnectionModal()}>
              Add a connection
            </button>
          </div>
        )}
        {profiles.map((p) => (
          <ProfileNode key={p.id} profile={p} setCtx={setCtx} />
        ))}
      </div>
      {ctx && (
        <div className="ctx-menu" style={{ left: ctx.x, top: ctx.y }}>
          {ctx.items.map((it, i) => (
            <button
              key={i}
              className={it.danger ? "danger" : undefined}
              onClick={() => {
                setCtx(null);
                it.action();
              }}
            >
              {it.label}
            </button>
          ))}
        </div>
      )}
    </>
  );
}

function ProfileNode({
  profile,
  setCtx,
}: {
  profile: ConnectionProfile;
  setCtx: (c: CtxMenu | null) => void;
}) {
  const expanded = useStore((s) => s.expandedProfiles[profile.id]);
  const toggleExpanded = useStore((s) => s.toggleExpanded);
  const catalog = useStore((s) => s.catalogs[profile.id]);
  const loading = useStore((s) => s.catalogLoading[profile.id]);
  const error = useStore((s) => s.catalogErrors[profile.id]);
  const loadCatalog = useStore((s) => s.loadCatalog);
  const openConnectionModal = useStore((s) => s.openConnectionModal);
  const openImport = useStore((s) => s.openImport);
  const addTab = useStore((s) => s.addTab);
  const sessions = useStore((s) => s.sessions);
  const disconnectProfile = useStore((s) => s.disconnectProfile);

  const openSessions = sessions[profile.id] ?? 0;
  const connected = openSessions > 0;

  const onContext = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const items: CtxMenu["items"] = [
      { label: "New console tab", action: () => addTab(profile.id) },
      { label: "Refresh schema", action: () => void loadCatalog(profile.id, true) },
      { label: "Import file…", action: () => openImport({ profileId: profile.id }) },
      { label: "Edit connection…", action: () => openConnectionModal(profile) },
    ];
    // Contextual: only offer disconnect when something is actually online.
    if (connected) {
      items.push({
        label: `Disconnect (${openSessions} session${openSessions === 1 ? "" : "s"})`,
        action: () => void disconnectProfile(profile.id),
      });
    }
    setCtx({ x: e.clientX, y: e.clientY, items });
  };

  return (
    <>
      <div
        className="tree-node"
        onClick={() => toggleExpanded(profile.id)}
        onContextMenu={onContext}
        title={`${profile.name} (${profile.driver}) — ${connected ? `${openSessions} open session(s)` : "offline"}`}
      >
        <Expander expanded={!!expanded} />
        <span className="icon">
          <DbIcon engine={engineOf(profile)} connected={connected} />
        </span>
        <span className="label">{profile.name}</span>
        <span className="meta">{profile.driver}</span>
        <span className="spacer" />
        {loading && <span className="spinner" />}
      </div>
      {expanded && (
        <>
          <ConsolesNode profileId={profile.id} />
          {error && (
            <div className="tree-node" style={{ paddingLeft: 30 }}>
              <span className="msg-err" style={{ fontSize: 12 }}>
                {error}
              </span>
            </div>
          )}
          {catalog?.schemas.map((schema) => (
            <SchemaNode
              key={schema.name}
              profileId={profile.id}
              schemaName={schema.name}
              tables={schema.tables}
              setCtx={setCtx}
            />
          ))}
        </>
      )}
    </>
  );
}

/** Stable fallback: a zustand selector must return a cached snapshot —
 * building `?? []` inside it yields a fresh array every call, which
 * useSyncExternalStore treats as an endless stream of new snapshots
 * (infinite re-render loop, blank window). */
const NO_CLOSED_CONSOLES: ConsoleFile[] = [];

/** Logical "Consoles" item: open tabs plus persisted (closed) consoles. */
function ConsolesNode({ profileId }: { profileId: string }) {
  const [open, setOpen] = useState(true);
  const tabs = useStore((s) => s.tabs);
  const activeTabId = useStore((s) => s.activeTabId);
  const setActiveTab = useStore((s) => s.setActiveTab);
  const closed = useStore((s) => s.closedConsoles[profileId]) ?? NO_CLOSED_CONSOLES;
  const reopenConsole = useStore((s) => s.reopenConsole);
  const deleteConsole = useStore((s) => s.deleteConsole);

  const live = tabs.filter((t) => t.profileId === profileId);
  const total = live.length + closed.length;

  return (
    <>
      <div className="tree-node" style={{ paddingLeft: 22 }} onClick={() => setOpen(!open)}>
        <Expander expanded={open} />
        <span className="icon">🗂</span>
        <span className="label">Consoles</span>
        <span className="meta">{total}</span>
      </div>
      {open &&
        live.map((t) => (
          <div
            key={t.id}
            className={`tree-node ${t.id === activeTabId ? "active-console" : ""}`}
            style={{ paddingLeft: 40 }}
            onClick={() => setActiveTab(t.id)}
            title={`${t.title} — open${t.database ? ` · ${t.database}` : ""}`}
          >
            <span className="icon">
              <ConsoleIcon open />
            </span>
            <span className="label">{t.title}</span>
            {t.database && <span className="meta">{t.database}</span>}
          </div>
        ))}
      {open &&
        closed.map((c) => (
          <div
            key={c.id}
            className="tree-node closed-console"
            style={{ paddingLeft: 40 }}
            onClick={() => reopenConsole(c)}
            title={`${c.title} — closed · click to reopen\n${c.sql.slice(0, 200)}`}
          >
            <span className="icon">
              <ConsoleIcon open={false} />
            </span>
            <span className="label">{c.title}</span>
            {c.database && <span className="meta">{c.database}</span>}
            <span className="spacer" />
            <span className="hover-actions">
              <button
                className="small ghost danger"
                title="Delete saved console (removes its log)"
                onClick={(e) => {
                  e.stopPropagation();
                  if (window.confirm(`Delete saved console "${c.title}" and its log?`)) {
                    void deleteConsole(profileId, c.id);
                  }
                }}
              >
                ✕
              </button>
            </span>
          </div>
        ))}
    </>
  );
}

function SchemaNode({
  profileId,
  schemaName,
  tables,
  setCtx,
}: {
  profileId: string;
  schemaName: string;
  tables: TableInfo[];
  setCtx: (c: CtxMenu | null) => void;
}) {
  const [open, setOpen] = useState(true);
  return (
    <>
      <div className="tree-node" style={{ paddingLeft: 22 }} onClick={() => setOpen(!open)}>
        <Expander expanded={open} />
        <span className="icon">▤</span>
        <span className="label">{schemaName}</span>
        <span className="meta">{tables.length}</span>
      </div>
      {open &&
        tables.map((t) => (
          <TableNode
            key={t.name}
            profileId={profileId}
            schemaName={schemaName}
            table={t}
            setCtx={setCtx}
          />
        ))}
    </>
  );
}

function TableNode({
  profileId,
  schemaName,
  table,
  setCtx,
}: {
  profileId: string;
  schemaName: string;
  table: TableInfo;
  setCtx: (c: CtxMenu | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const addTab = useStore((s) => s.addTab);
  const openDdl = useStore((s) => s.openDdl);
  const openImport = useStore((s) => s.openImport);
  const openExport = useStore((s) => s.openExport);
  const profiles = useStore((s) => s.profiles);

  const qualified = quoteFor(profiles.find((p) => p.id === profileId)?.driver, schemaName, table.name);
  const selectAll = `SELECT * FROM ${qualified}`;

  const onDouble = () => {
    addTab(profileId, `${selectAll};`, true);
  };

  const onContext = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setCtx({
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: "Open data (SELECT *)", action: onDouble },
        {
          label: "Show DDL",
          action: () => openDdl({ profileId, schema: schemaName, table: table.name }),
        },
        {
          label: "Export table…",
          action: () => openExport({ profileId, sql: selectAll, suggestedName: table.name }),
        },
        {
          label: "Import into this table…",
          action: () => openImport({ profileId, schema: schemaName, table: table.name }),
        },
      ],
    });
  };

  return (
    <>
      <div
        className="tree-node"
        style={{ paddingLeft: 40 }}
        onClick={() => setOpen(!open)}
        onDoubleClick={onDouble}
        onContextMenu={onContext}
        title={`${table.name} — double-click to open data, right-click for actions`}
      >
        <Expander expanded={open} />
        <span className="icon">{table.kind === "view" || table.kind === "materialized view" ? <ViewIcon /> : <TableIcon />}</span>
        <span className="label">{table.name}</span>
        {table.kind !== "table" && <span className="meta">{table.kind}</span>}
      </div>
      {open &&
        table.columns.map((c) => (
          <div className="tree-node" style={{ paddingLeft: 62, cursor: "default" }} key={c.name}>
            <span
              className="icon"
              style={{ color: table.pk.includes(c.name) ? "var(--warn)" : "var(--text-dim)", fontSize: 10 }}
            >
              {table.pk.includes(c.name) ? "🔑" : "●"}
            </span>
            <span className="label">{c.name}</span>
            <span className="meta">
              {c.dataType}
              {c.nullable ? "" : " not null"}
            </span>
          </div>
        ))}
    </>
  );
}

function quoteFor(driver: string | undefined, schema: string, table: string): string {
  const q = (s: string) =>
    driver === "mysql" ? `\`${s.replace(/`/g, "``")}\`` : `"${s.replace(/"/g, '""')}"`;
  const plain = (s: string) => /^[a-z_][a-z0-9_]*$/.test(s);
  const part = (s: string) => (plain(s) ? s : q(s));
  if (!schema || schema === "main" || schema === "(default)") return part(table);
  return `${part(schema)}.${part(table)}`;
}

// referenced for tree section typing
export type { ConsoleFile };
