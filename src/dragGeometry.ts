/**
 * Pure drag geometry for the card drag-to-group gesture — SPEC.md §11 Motion, plan 030.
 *
 * ZERO IMPORTS — a hard requirement, not a style note. `src/store.ts` imports `./api`, which
 * imports `@tauri-apps/api`, which `node --test` cannot resolve under this project's
 * `moduleResolution: "bundler"`. This leaf module is the only part of the drag feature a machine
 * can verify (see `dragGeometry.test.mjs`), so it must stay reachable without a transpiler.
 */

/** Dwell time before a hovered target arms (§11 Motion: "drag-to-group feedback"). */
export const DWELL_MS = 450;
/** How far the pointer may wander during the dwell without restarting the timer. */
export const DWELL_SLOP_PX = 6;
/** Minimum pointer travel from `pointerdown` before a candidate becomes a real drag. */
export const DRAG_THRESHOLD_PX = 5;

export interface Point {
  x: number;
  y: number;
}

export interface DragRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface DragTile {
  id: string;
  kind: "project" | "folder";
  rect: DragRect;
}

export type DropIntent =
  | { kind: "none" }
  | { kind: "merge"; targetKind: "project" | "folder"; targetId: string }
  // Reserved for §16's parked drag-to-reorder (drop in the gap between tiles). `hitTest` below
  // must NEVER construct this variant today — it exists only so reorder can land later without
  // making today's folder-merge drags ambiguous.
  | { kind: "insert"; beforeId: string | null };

function pointInRect(point: Point, rect: DragRect): boolean {
  return (
    point.x >= rect.left && point.x <= rect.right && point.y >= rect.top && point.y <= rect.bottom
  );
}

/**
 * v1 hit test: `merge` when `point` falls inside a tile that is not `sourceId`, `none` otherwise —
 * including the gutter between tiles and the drag source's own tile. Never returns `insert`.
 */
export function hitTest(point: Point, sourceId: string, tiles: readonly DragTile[]): DropIntent {
  for (const tile of tiles) {
    if (tile.id === sourceId) continue;
    if (pointInRect(point, tile.rect)) {
      return { kind: "merge", targetKind: tile.kind, targetId: tile.id };
    }
  }
  return { kind: "none" };
}

/** True once the pointer has travelled at least `DRAG_THRESHOLD_PX` from `origin`. */
export function hasMovedEnough(origin: Point, point: Point): boolean {
  return Math.hypot(point.x - origin.x, point.y - origin.y) >= DRAG_THRESHOLD_PX;
}

/** True while `point` stays within `DWELL_SLOP_PX` of the dwell `anchor`. */
export function stillWithinSlop(anchor: Point, point: Point): boolean {
  return Math.hypot(point.x - anchor.x, point.y - anchor.y) <= DWELL_SLOP_PX;
}
