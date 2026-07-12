/** Geometry for one reorder candidate: its id and its midpoint along the
 * drag axis (clientX for the tab strip, clientY for the connection list). */
export interface DragItem {
  id: string;
  mid: number;
}

/**
 * Insertion index for a hover-to-reorder gesture, computed from the OTHER
 * items only — the dragged item is skipped in the hit-test.
 *
 * Why exclude it: while dragging, the lifted element stays in normal flow, so
 * every reorder physically relocates it (often under the pointer). Including
 * it in the hit-test makes the next pointermove compute a different slot and
 * reorder again — an oscillation that reads as rapid flicker. Measuring only
 * against the stationary neighbours makes the target a stable fixpoint.
 *
 * The returned value is the index in the array once the dragged item has been
 * removed, i.e. exactly the `to` for a `splice(from, 1)` + `splice(to, 0)`
 * store update — so there is no off-by-one when dragging downward.
 */
export function dropIndex(items: DragItem[], draggedId: string, pointer: number): number {
  let idx = 0;
  for (const it of items) {
    if (it.id === draggedId) continue;
    if (pointer < it.mid) break;
    idx += 1;
  }
  return idx;
}
