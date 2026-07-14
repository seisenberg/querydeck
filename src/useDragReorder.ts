import { useLayoutEffect, useRef, useState } from "react";
import { dropIndex } from "./dragorder";

/**
 * Pointer-driven drag-to-reorder for items laid out along one axis inside a
 * scroll container (console tabs: x, connection rows: y).
 *
 * Interaction model (Chrome-tab style):
 * - The lifted item follows the pointer along the axis via an inline
 *   transform, so the user carries it instead of watching it snap into slots.
 * - The other items reorder live around it and FLIP from their old slot to
 *   the new one, which leaves a visible gap where the lifted item will land.
 * - On release the lifted item animates from wherever the pointer left it
 *   into its resting slot.
 *
 * Hit-testing uses offsetLeft/offsetTop — resting layout geometry, immune to
 * the in-flight transforms — via the shared dropIndex helper; see dragorder.ts
 * for why the dragged item is excluded from the hit-test.
 */
export interface DragReorderOptions {
  axis: "x" | "y";
  /** Scroll container. Must be position:relative so it is the offsetParent
   * of the items (offset geometry is measured against it). */
  containerRef: React.RefObject<HTMLElement | null>;
  /** Selector matching the reorderable items inside the container. */
  itemSelector: string;
  /** dataset property carrying the item id (e.g. "tabId" for data-tab-id). */
  idKey: string;
  /** Store reorder: move item `id` to `toIndex` (splice-out/splice-in). */
  onMove: (id: string, toIndex: number) => void;
  /** Fires on release of a drag that actually moved (not a plain click). */
  onDrop?: () => void;
}

export function useDragReorder({
  axis,
  containerRef,
  itemSelector,
  idKey,
  onMove,
  onDrop,
}: DragReorderOptions) {
  const horiz = axis === "x";

  const drag = useRef<{
    id: string;
    startClient: number;
    /** Pointer-to-item-origin distance at grab, so the item doesn't jump. */
    grabOffset: number;
    /** Latest pointer position in container content coordinates. */
    lastContent: number;
    moved: boolean;
  } | null>(null);
  // State (not just the ref) so lift/drop re-render and toggle the item class.
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const positions = useRef<Map<string, number>>(new Map());

  const itemNodes = () =>
    Array.from(containerRef.current?.querySelectorAll<HTMLElement>(itemSelector) ?? []);
  const nodeOf = (id: string) => itemNodes().find((n) => n.dataset[idKey] === id) ?? null;
  const offsetOf = (n: HTMLElement) => (horiz ? n.offsetLeft : n.offsetTop);
  const sizeOf = (n: HTMLElement) => (horiz ? n.offsetWidth : n.offsetHeight);

  /** Pointer position in the container's content box (scroll-adjusted). */
  const contentCoord = (e: { clientX: number; clientY: number }) => {
    const cont = containerRef.current!;
    const rect = cont.getBoundingClientRect();
    return horiz
      ? e.clientX - rect.left - cont.clientLeft + cont.scrollLeft
      : e.clientY - rect.top - cont.clientTop + cont.scrollTop;
  };

  /** Glue the lifted item to the pointer: translate it from its resting slot
   * toward the grab point, clamped inside the container's content box. */
  const followPointer = (n: HTMLElement) => {
    const d = drag.current;
    const cont = containerRef.current;
    if (!d || !cont) return;
    const limit = (horiz ? cont.scrollWidth : cont.scrollHeight) - sizeOf(n);
    const desired = Math.max(0, Math.min(limit, d.lastContent - d.grabOffset));
    n.style.transform = `translate${horiz ? "X" : "Y"}(${desired - offsetOf(n)}px)`;
  };

  // FLIP displaced items from their old slot to the new one. The lifted item
  // is re-glued to the pointer instead: a reorder render changes its
  // offsetLeft/Top, and without this it would jump until the next pointermove.
  useLayoutEffect(() => {
    const prev = positions.current;
    const next = new Map<string, number>();
    itemNodes().forEach((n) => {
      const id = n.dataset[idKey];
      if (!id) return;
      const off = offsetOf(n);
      next.set(id, off);
      if (id === drag.current?.id) {
        if (drag.current.moved) followPointer(n);
        return;
      }
      const old = prev.get(id);
      if (old != null && old !== off) {
        n.animate(
          [{ transform: `translate${horiz ? "X" : "Y"}(${old - off}px)` }, { transform: "none" }],
          { duration: 140, easing: "ease-out" },
        );
      }
    });
    positions.current = next;
  });

  const onPointerDown = (id: string) => (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const n = e.currentTarget as HTMLElement;
    drag.current = {
      id,
      startClient: horiz ? e.clientX : e.clientY,
      grabOffset: contentCoord(e) - offsetOf(n),
      lastContent: 0,
      moved: false,
    };
    n.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    const cont = containerRef.current;
    if (!d || !(e.buttons & 1) || !cont) return;
    const client = horiz ? e.clientX : e.clientY;
    if (!d.moved && Math.abs(client - d.startClient) < 5) return;
    if (!d.moved) {
      d.moved = true;
      setDraggingId(d.id);
    }
    e.preventDefault();
    d.lastContent = contentCoord(e);
    const node = nodeOf(d.id);
    if (node) followPointer(node);
    const items = itemNodes().map((n) => ({
      id: n.dataset[idKey] ?? "",
      mid: offsetOf(n) + sizeOf(n) / 2,
    }));
    const from = items.findIndex((it) => it.id === d.id);
    const to = dropIndex(items, d.id, d.lastContent);
    if (to !== from) onMove(d.id, to);
  };

  const onPointerEnd = () => {
    const d = drag.current;
    drag.current = null;
    if (d?.moved) {
      const n = nodeOf(d.id);
      if (n && n.style.transform) {
        // Settle: drop the inline transform and animate from where the
        // pointer left the item into its resting slot.
        const lifted = n.style.transform;
        n.style.transform = "";
        n.animate([{ transform: lifted }, { transform: "none" }], {
          duration: 160,
          easing: "ease-out",
        });
      }
      onDrop?.();
    }
    setDraggingId(null);
  };

  return { draggingId, onPointerDown, onPointerMove, onPointerEnd };
}
