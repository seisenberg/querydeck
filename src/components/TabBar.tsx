import { useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useStore } from "../store";

/** Shared mousedown handler for window drag regions (sidebar header, tab
 * strip). Manual startDragging is used instead of data-tauri-drag-region so
 * behavior is explicit: primary button on the region itself (not a child)
 * drags; double-click zooms, matching native titlebars. */
export function dragRegionMouseDown(e: React.MouseEvent) {
  if (e.button !== 0 || e.target !== e.currentTarget) return;
  e.preventDefault();
  const win = getCurrentWindow();
  if (e.detail === 2) void win.toggleMaximize();
  else void win.startDragging();
}

export default function TabBar() {
  const tabs = useStore((s) => s.tabs);
  const activeTabId = useStore((s) => s.activeTabId);
  const setActiveTab = useStore((s) => s.setActiveTab);
  const closeTab = useStore((s) => s.closeTab);
  const moveTab = useStore((s) => s.moveTab);
  const setConnectionPicker = useStore((s) => s.setConnectionPicker);
  const profiles = useStore((s) => s.profiles);

  const barRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ id: string; startX: number; moved: boolean } | null>(null);

  const onTabPointerDown = (id: string) => (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button")) return; // close button
    setActiveTab(id);
    drag.current = { id, startX: e.clientX, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };

  const onTabPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d || !(e.buttons & 1) || !barRef.current) return;
    if (!d.moved && Math.abs(e.clientX - d.startX) < 5) return;
    d.moved = true;
    e.preventDefault();
    // Target slot: first tab whose midpoint is right of the pointer.
    const nodes = Array.from(barRef.current.querySelectorAll<HTMLElement>(".tab"));
    let to = nodes.findIndex((n) => {
      const r = n.getBoundingClientRect();
      return e.clientX < r.left + r.width / 2;
    });
    if (to === -1) to = nodes.length - 1;
    moveTab(d.id, to);
  };

  const onTabPointerUp = () => {
    drag.current = null;
  };

  return (
    <div className="tabbar" ref={barRef} onMouseDown={dragRegionMouseDown}>
      {tabs.map((t) => {
        const prof = profiles.find((p) => p.id === t.profileId);
        return (
          <div
            key={t.id}
            className={`tab ${t.id === activeTabId ? "active" : ""} ${
              drag.current?.id === t.id && drag.current.moved ? "dragging" : ""
            }`}
            onPointerDown={onTabPointerDown(t.id)}
            onPointerMove={onTabPointerMove}
            onPointerUp={onTabPointerUp}
            onAuxClick={(e) => {
              if (e.button === 1) closeTab(t.id);
            }}
            title={prof ? `${t.title} — ${prof.name}` : t.title}
          >
            {t.running && <span className="spinner" />}
            <span>{t.title}</span>
            {prof && <span style={{ color: "var(--text-dim)", fontSize: 11 }}>{prof.name}</span>}
            <button
              className="close"
              onClick={(e) => {
                e.stopPropagation();
                closeTab(t.id);
              }}
              title="Close tab"
            >
              ×
            </button>
          </div>
        );
      })}
      <button
        className="new-tab"
        onClick={() => setConnectionPicker(true)}
        title="New console tab (⌘T) — pick a connection"
      >
        +
      </button>
    </div>
  );
}
