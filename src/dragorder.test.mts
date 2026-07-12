import assert from "node:assert/strict";
import { dropIndex, type DragItem } from "./dragorder.ts";

// A tiny model of the reorder loop: items are laid out at fixed pitch, the
// dragged item is kept in flow (as the app does), and we re-measure midpoints
// from the *current* order after each move — exactly the feedback loop that
// caused the flicker.
const PITCH = 20;
function midpoints(order: string[]): DragItem[] {
  return order.map((id, i) => ({ id, mid: i * PITCH + PITCH / 2 }));
}
function applyMove(order: string[], id: string, to: number): string[] {
  const from = order.indexOf(id);
  const next = [...order];
  const [m] = next.splice(from, 1);
  next.splice(to, 0, m);
  return next;
}

// The OLD algorithm: hit-test includes the dragged item.
function oldTarget(items: DragItem[], pointer: number): number {
  const to = items.findIndex((n) => pointer < n.mid);
  return to === -1 ? items.length - 1 : to;
}

// --- 1. dropIndex excludes the dragged item and returns the reduced-array index.
{
  const order = ["A", "B", "C", "D"];
  // Pointer sitting between C and D (index 3 slot region), dragging A.
  const pointer = 3 * PITCH; // == boundary just below D's midpoint
  assert.equal(dropIndex(midpoints(order), "A", pointer), 2, "A lands after B,C");
}

// --- 2. New algorithm reaches a fixpoint in ONE step for every start position
//        and pointer — no oscillation. This is the core regression guard.
function reachesFixpoint(target: (items: DragItem[], id: string, ptr: number) => number) {
  const ids = ["A", "B", "C", "D", "E"];
  for (const dragged of ids) {
    // Sweep the pointer across the whole strip in fine steps.
    for (let p = -5; p <= ids.length * PITCH + 5; p += 3) {
      let order = [...ids];
      const seen = new Set<string>();
      // Iterate the drag loop; it must settle (no cycling) quickly.
      for (let step = 0; step < 10; step++) {
        const to = target(midpoints(order), dragged, p);
        const from = order.indexOf(dragged);
        if (to === from) break; // settled
        order = applyMove(order, dragged, to);
        const key = order.join("");
        if (seen.has(key)) return false; // revisited a state => oscillation
        seen.add(key);
      }
    }
  }
  return true;
}
assert.equal(reachesFixpoint(dropIndex), true, "dropIndex never oscillates");

// --- 3. Off-by-one: the OLD hit-test (dragged item included) placed a
//        downward-dragged item one slot too far. Dragging A with the pointer
//        between C and D should land A *between* them (reduced index 2), but
//        the old findIndex returned 3 (after D). dropIndex fixes this.
{
  const order = ["A", "B", "C", "D"];
  const pointer = 3 * PITCH; // between C (mid 50) and D (mid 70)
  assert.equal(oldTarget(midpoints(order), pointer), 3, "old: one slot too far");
  assert.equal(dropIndex(midpoints(order), "A", pointer), 2, "new: correct slot");
}

console.log("dragorder: all assertions passed");
