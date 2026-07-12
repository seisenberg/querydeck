import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { ConnectionProfile, ConsoleFile, DbObject, TableInfo } from "../api";
import { useStore } from "../store";
import { ConsoleIcon, DbIcon, Expander, TableIcon, ViewIcon, engineOf } from "./DbIcons";
import { dragRegionMouseDown } from "./TabBar";

interface CtxMenu {
  x: number;
  y: number;
  items: { label: string; danger?: boolean; action: () => void }[];
}

/** Drag-to-reorder plumbing handed down to each profile row. */
export interface ProfileDrag {
  draggingId: string | null;
  onPointerDown: (id: string) => (e: React.PointerEvent) => void;
  onPointerMove: (e: React.PointerEvent) => void;
  onPointerUp: () => void;
  /** True exactly once right after a drag ended — swallow the click. */
  consumeClickSuppression: () => boolean;
}

export default function SchemaTree() {
  const profiles = useStore((s) => s.profiles);
  const openConnectionModal = useStore((s) => s.openConnectionModal);
  const toggleSidebar = useStore((s) => s.toggleSidebar);
  const moveProfile = useStore((s) => s.moveProfile);
  const persistProfileOrder = useStore((s) => s.persistProfileOrder);
  const [ctx, setCtx] = useState<CtxMenu | null>(null);

  const treeRef = useRef<HTMLDivElement>(null);
  const dragProf = useRef<{ id: string; startY: number; moved: boolean } | null>(null);
  const suppressClick = useRef(false);
  const [draggingProfileId, setDraggingProfileId] = useState<string | null>(null);
  const profPositions = useRef<Map<string, number>>(new Map());

  // FLIP: displaced connection rows slide to their new slot on reorder.
  useLayoutEffect(() => {
    const prev = profPositions.current;
    const next = new Map<string, number>();
    const nodes = treeRef.current?.querySelectorAll<HTMLElement>("[data-profile-id]") ?? [];
    nodes.forEach((n) => {
      const id = n.dataset.profileId;
      if (!id) return;
      next.set(id, n.offsetTop);
      const old = prev.get(id);
      if (old != null && old !== n.offsetTop && id !== dragProf.current?.id) {
        n.animate(
          [{ transform: `translateY(${old - n.offsetTop}px)` }, { transform: "translateY(0)" }],
          { duration: 140, easing: "ease-out" },
        );
      }
    });
    profPositions.current = next;
  });

  const profileDrag: ProfileDrag = {
    draggingId: draggingProfileId,
    onPointerDown: (id) => (e) => {
      if (e.button !== 0) return;
      dragProf.current = { id, startY: e.clientY, moved: false };
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    },
    onPointerMove: (e) => {
      const d = dragProf.current;
      if (!d || !(e.buttons & 1) || !treeRef.current) return;
      if (!d.moved && Math.abs(e.clientY - d.startY) < 5) return;
      if (!d.moved) {
        d.moved = true;
        setDraggingProfileId(d.id);
      }
      e.preventDefault();
      // Target slot: first connection row whose midpoint is below the pointer.
      const nodes = Array.from(
        treeRef.current.querySelectorAll<HTMLElement>("[data-profile-id]"),
      );
      let to = nodes.findIndex((n) => {
        const r = n.getBoundingClientRect();
        return e.clientY < r.top + r.height / 2;
      });
      if (to === -1) to = nodes.length - 1;
      moveProfile(d.id, to);
    },
    onPointerUp: () => {
      if (dragProf.current?.moved) {
        suppressClick.current = true;
        persistProfileOrder();
      }
      dragProf.current = null;
      setDraggingProfileId(null);
    },
    consumeClickSuppression: () => {
      const s = suppressClick.current;
      suppressClick.current = false;
      return s;
    },
  };

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
      <div className="sidebar-header" onMouseDown={dragRegionMouseDown}>
        <span className="sidebar-title">Connections</span>
        <span className="actions">
          <button className="small ghost" title="New connection" onClick={() => openConnectionModal()}>
            +
          </button>
          <button className="small ghost" title="Hide connections" onClick={toggleSidebar}>
            «
          </button>
        </span>
      </div>
      <div className="tree" ref={treeRef}>
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
          <ProfileNode key={p.id} profile={p} setCtx={setCtx} drag={profileDrag} />
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

/** Memoized: SchemaTree re-renders whenever the context menu opens/closes;
 * the per-connection subtrees only need to follow their own store slices. */
const ProfileNode = memo(function ProfileNode({
  profile,
  setCtx,
  drag,
}: {
  profile: ConnectionProfile;
  setCtx: (c: CtxMenu | null) => void;
  drag: ProfileDrag;
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
  // Primitive selector: a sessions-map refresh only re-renders this node
  // when THIS connection's count actually changed.
  const openSessions = useStore((s) => s.sessions[profile.id] ?? 0);
  const disconnectProfile = useStore((s) => s.disconnectProfile);

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
        className={`tree-node ${drag.draggingId === profile.id ? "dragging-profile" : ""}`}
        data-profile-id={profile.id}
        onClick={() => {
          if (drag.consumeClickSuppression()) return;
          toggleExpanded(profile.id);
        }}
        onPointerDown={drag.onPointerDown(profile.id)}
        onPointerMove={drag.onPointerMove}
        onPointerUp={drag.onPointerUp}
        onPointerCancel={drag.onPointerUp}
        onContextMenu={onContext}
        title={`${profile.name} (${profile.driver}) — ${connected ? `${openSessions} open session(s)` : "offline"} · drag to reorder`}
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
          <ConsolesNode profileId={profile.id} setCtx={setCtx} />
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
          {catalog && <OtherObjectsNode profileId={profile.id} setCtx={setCtx} />}
        </>
      )}
    </>
  );
});

const KIND_LABELS: Record<string, string> = {
  function: "Functions",
  procedure: "Procedures",
  sequence: "Sequences",
  trigger: "Triggers",
  role: "Roles",
};
const KIND_ORDER = ["function", "procedure", "sequence", "trigger", "role"];

/** Rarely-needed schema objects, tucked away collapsed: functions, stored
 * procedures, sequences, triggers, roles. Right-click exposes the DDL. */
function OtherObjectsNode({
  profileId,
  setCtx,
}: {
  profileId: string;
  setCtx: (c: CtxMenu | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const objects = useStore((s) => s.objectsByProfile[profileId]);
  const loading = useStore((s) => s.objectsLoading[profileId]);
  const loadObjects = useStore((s) => s.loadObjects);
  const openDdl = useStore((s) => s.openDdl);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next && objects === undefined) void loadObjects(profileId);
  };

  const objectCtx = (o: DbObject) => (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setCtx({
      x: e.clientX,
      y: e.clientY,
      items: [
        {
          label: "Show DDL",
          action: () =>
            openDdl({
              profileId,
              schema: o.schema ?? "",
              table: o.name,
              kind: o.kind,
              detail: o.detail ?? "",
            }),
        },
      ],
    });
  };

  const groups = KIND_ORDER.map(
    (k) => [k, (objects ?? []).filter((o) => o.kind === k)] as const,
  ).filter(([, list]) => list.length > 0);

  return (
    <>
      <div className="tree-node" style={{ paddingLeft: 22 }} onClick={toggle}>
        <Expander expanded={open} />
        <span className="icon">⚙</span>
        <span className="label">Other objects</span>
        {objects !== undefined && <span className="meta">{objects.length}</span>}
        {loading && <span className="spinner" />}
      </div>
      {open && objects !== undefined && objects.length === 0 && !loading && (
        <div className="tree-node" style={{ paddingLeft: 40, cursor: "default" }}>
          <span className="meta">none found</span>
        </div>
      )}
      {open &&
        groups.map(([kind, list]) => (
          <div key={kind}>
            <div className="tree-node" style={{ paddingLeft: 40, cursor: "default" }}>
              <span className="meta">{KIND_LABELS[kind]}</span>
            </div>
            {list.map((o, i) => (
              <div
                key={`${o.schema ?? ""}.${o.name}.${i}`}
                className="tree-node"
                style={{ paddingLeft: 56 }}
                onContextMenu={objectCtx(o)}
                onDoubleClick={() =>
                  openDdl({
                    profileId,
                    schema: o.schema ?? "",
                    table: o.name,
                    kind: o.kind,
                    detail: o.detail ?? "",
                  })
                }
                title={`${o.schema ? `${o.schema}.` : ""}${o.name}${
                  o.detail ? ` (${o.detail})` : ""
                } — right-click for DDL`}
              >
                <span className="icon" style={{ color: "var(--text-dim)", fontSize: 10 }}>
                  ƒ
                </span>
                <span className="label">{o.name}</span>
                {o.detail && kind !== "trigger" && (
                  <span className="meta" style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
                    {o.detail}
                  </span>
                )}
              </div>
            ))}
          </div>
        ))}
    </>
  );
}

/** Stable fallback: a zustand selector must return a cached snapshot —
 * building `?? []` inside it yields a fresh array every call, which
 * useSyncExternalStore treats as an endless stream of new snapshots
 * (infinite re-render loop, blank window). */
const NO_CLOSED_CONSOLES: ConsoleFile[] = [];

/** Logical "Consoles" item: open tabs plus persisted (closed) consoles. */
function ConsolesNode({
  profileId,
  setCtx,
}: {
  profileId: string;
  setCtx: (c: CtxMenu | null) => void;
}) {
  const [open, setOpen] = useState(true);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const tabs = useStore((s) => s.tabs);
  const activeTabId = useStore((s) => s.activeTabId);
  const setActiveTab = useStore((s) => s.setActiveTab);
  const closed = useStore((s) => s.closedConsoles[profileId]) ?? NO_CLOSED_CONSOLES;
  const reopenConsole = useStore((s) => s.reopenConsole);
  const deleteConsole = useStore((s) => s.deleteConsole);
  const closeTab = useStore((s) => s.closeTab);
  const renameConsole = useStore((s) => s.renameConsole);

  const live = tabs.filter((t) => t.profileId === profileId);
  const total = live.length + closed.length;

  const renameInput = (id: string, current: string) => (
    <input
      autoFocus
      defaultValue={current}
      className="rename-input"
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          renameConsole(id, (e.target as HTMLInputElement).value);
          setRenamingId(null);
        } else if (e.key === "Escape") {
          setRenamingId(null);
        }
      }}
      onBlur={() => setRenamingId(null)}
    />
  );

  const consoleCtx =
    (id: string, title: string, isLive: boolean, file?: ConsoleFile) =>
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const items: CtxMenu["items"] = [
        isLive
          ? { label: "Show console", action: () => setActiveTab(id) }
          : { label: "Reopen console", action: () => file && reopenConsole(file) },
        { label: "Rename…", action: () => setRenamingId(id) },
      ];
      if (isLive) {
        items.push({ label: "Close console", action: () => closeTab(id) });
      } else {
        items.push({
          label: "Delete saved console…",
          danger: true,
          action: () => {
            if (window.confirm(`Delete saved console "${title}" and its log?`)) {
              void deleteConsole(profileId, id);
            }
          },
        });
      }
      setCtx({ x: e.clientX, y: e.clientY, items });
    };

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
            onContextMenu={consoleCtx(t.id, t.title, true)}
            title={`${t.title} — open · right-click to rename`}
          >
            <span className="icon">
              <ConsoleIcon open />
            </span>
            {renamingId === t.id ? (
              renameInput(t.id, t.title)
            ) : (
              <span className="label">{t.title}</span>
            )}
          </div>
        ))}
      {open &&
        closed.map((c) => (
          <div
            key={c.id}
            className="tree-node closed-console"
            style={{ paddingLeft: 40 }}
            onClick={() => reopenConsole(c)}
            onContextMenu={consoleCtx(c.id, c.title, false, c)}
            title={`${c.title} — closed · click to reopen\n${c.sql.slice(0, 200)}`}
          >
            <span className="icon">
              <ConsoleIcon open={false} />
            </span>
            {renamingId === c.id ? (
              renameInput(c.id, c.title)
            ) : (
              <span className="label">{c.title}</span>
            )}
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
