import { useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useStore } from "../store";
import { useDragReorder } from "../useDragReorder";

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
  const newConsoleRequested = useStore((s) => s.newConsoleRequested);
  const profiles = useStore((s) => s.profiles);

  const scrollRef = useRef<HTMLDivElement>(null);
  const reorder = useDragReorder({
    axis: "x",
    containerRef: scrollRef,
    itemSelector: ".tab",
    idKey: "tabId",
    onMove: moveTab,
  });

  const onTabPointerDown = (id: string) => (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button")) return; // close button
    setActiveTab(id);
    reorder.onPointerDown(id)(e);
  };

  return (
    <div className="tabbar" onMouseDown={dragRegionMouseDown}>
      {/* Anchored left so it can never be pushed off screen by many tabs. */}
      <button
        className="new-tab"
        onClick={() => newConsoleRequested()}
        title="Open console (⌘T)"
      >
        +
      </button>
      <div className="tabs-scroll" ref={scrollRef} onMouseDown={dragRegionMouseDown}>
        {tabs.map((t) => {
          const prof = profiles.find((p) => p.id === t.profileId);
          return (
            <div
              key={t.id}
              data-tab-id={t.id}
              className={`tab ${t.id === activeTabId ? "active" : ""} ${
                reorder.draggingId === t.id ? "dragging" : ""
              }`}
              onPointerDown={onTabPointerDown(t.id)}
              onPointerMove={reorder.onPointerMove}
              onPointerUp={reorder.onPointerEnd}
              onPointerCancel={reorder.onPointerEnd}
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
      </div>
    </div>
  );
}
