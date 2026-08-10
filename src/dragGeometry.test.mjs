// Tests for the zero-import leaf module `dragGeometry.ts` — plan 030. Imported by its `.ts` path
// directly: Node v24's built-in type-stripping runs this with no transpiler and no dependency
// (confirmed working before writing the rest of this plan — see the executor report).
import test from "node:test";
import assert from "node:assert/strict";
import {
  DRAG_THRESHOLD_PX,
  DWELL_SLOP_PX,
  hasMovedEnough,
  hitTest,
  stillWithinSlop,
} from "./dragGeometry.ts";

const cardRect = { left: 0, top: 0, right: 100, bottom: 100 };
const folderRect = { left: 200, top: 0, right: 300, bottom: 100 };
const tiles = [
  { id: "source", kind: "project", rect: cardRect },
  { id: "target-card", kind: "project", rect: { left: 120, top: 0, right: 220, bottom: 100 } },
  { id: "target-folder", kind: "folder", rect: folderRect },
];

test("point inside a non-source tile merges with that tile's id and kind", () => {
  const result = hitTest({ x: 150, y: 50 }, "source", tiles);
  assert.deepEqual(result, { kind: "merge", targetKind: "project", targetId: "target-card" });
});

test("point inside the source's own tile is none", () => {
  const result = hitTest({ x: 50, y: 50 }, "source", tiles);
  assert.deepEqual(result, { kind: "none" });
});

test("point in the gutter between tiles is none, not insert", () => {
  const result = hitTest({ x: 110, y: 50 }, "source", tiles);
  assert.deepEqual(result, { kind: "none" });
});

test("point over a folder tile merges with targetKind folder", () => {
  const result = hitTest({ x: 250, y: 50 }, "source", tiles);
  assert.deepEqual(result, { kind: "merge", targetKind: "folder", targetId: "target-folder" });
});

test("hasMovedEnough is false below the threshold and true at/above it", () => {
  const origin = { x: 0, y: 0 };
  assert.equal(hasMovedEnough(origin, { x: 4, y: 0 }), false);
  assert.equal(hasMovedEnough(origin, { x: 6, y: 0 }), true);
  assert.equal(DRAG_THRESHOLD_PX, 5);
});

test("stillWithinSlop is true at the slop boundary and false past it", () => {
  const anchor = { x: 0, y: 0 };
  assert.equal(stillWithinSlop(anchor, { x: 5, y: 0 }), true);
  assert.equal(stillWithinSlop(anchor, { x: 7, y: 0 }), false);
  assert.equal(DWELL_SLOP_PX, 6);
});
