import { useLayoutEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useStore } from "../store";
import { dropIndex } from "../dragorder";

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
  const drag = useRef<{ id: string; startX: number; moved: boolean } | null>(null);
  // State (not just the ref) so the drop re-renders and clears the lifted
  // style — previously the class lingered because nothing re-rendered.
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const positions = useRef<Map<string, number>>(new Map());

  // FLIP: when reordering displaces tabs, slide them from their old slot to
  // the new one. offsetLeft is scroll-independent. The dragged tab itself is
  // skipped — it should snap to the slot under the pointer.
  useLayoutEffect(() => {
    const prev = positions.current;
    const next = new Map<string, number>();
    const nodes = scrollRef.current?.querySelectorAll<HTMLElement>(".tab") ?? [];
    nodes.forEach((n) => {
      const id = n.dataset.tabId;
      if (!id) return;
      next.set(id, n.offsetLeft);
      const old = prev.get(id);
      if (old != null && old !== n.offsetLeft && id !== drag.current?.id) {
        n.animate(
          [{ transform: `translateX(${old - n.offsetLeft}px)` }, { transform: "translateX(0)" }],
          { duration: 140, easing: "ease-out" },
        );
      }
    });
    positions.current = next;
  });

  const onTabPointerDown = (id: string) => (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button")) return; // close button
    setActiveTab(id);
    drag.current = { id, startX: e.clientX, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };

  const onTabPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d || !(e.buttons & 1) || !scrollRef.current) return;
    if (!d.moved && Math.abs(e.clientX - d.startX) < 5) return;
    if (!d.moved) {
      d.moved = true;
      setDraggingId(d.id);
    }
    e.preventDefault();
    // Target slot from transform-independent layout geometry (offsetLeft),
    // measuring only the *other* tabs. getBoundingClientRect would report the
    // in-flight FLIP transform, so mid-animation reads move the midpoints and
    // the target oscillates — the flicker. offsetLeft is the resting layout
    // position, immune to transforms; the scroll container is its offsetParent.
    const cont = scrollRef.current;
    const contRect = cont.getBoundingClientRect();
    const pointer = e.clientX - contRect.left - cont.clientLeft + cont.scrollLeft;
    const items = Array.from(cont.querySelectorAll<HTMLElement>(".tab")).map((n) => ({
      id: n.dataset.tabId ?? "",
      mid: n.offsetLeft + n.offsetWidth / 2,
    }));
    const from = items.findIndex((it) => it.id === d.id);
    const to = dropIndex(items, d.id, pointer);
    if (to !== from) moveTab(d.id, to);
  };

  const endDrag = () => {
    drag.current = null;
    setDraggingId(null);
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
                draggingId === t.id ? "dragging" : ""
              }`}
              onPointerDown={onTabPointerDown(t.id)}
              onPointerMove={onTabPointerMove}
              onPointerUp={endDrag}
              onPointerCancel={endDrag}
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
