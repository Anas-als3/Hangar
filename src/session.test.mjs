// Tests for the zero-import leaf module `session.ts` — plan 051. Imported by its `.ts` path
// directly: Node v24's built-in type-stripping runs this with no transpiler and no dependency,
// same arrangement as `dragGeometry.test.mjs` (plan 030).
import test from "node:test";
import assert from "node:assert/strict";
import { lastSessionCluster, stackInventory } from "./session.ts";

const iso = (msAgo) => new Date(Date.now() - msAgo).toISOString();

test("two projects seconds apart plus one six hours earlier -> the two", () => {
  const projects = [
    { id: "a", name: "A", status: "stopped", path: "/a", lastRunAt: iso(7_000) },
    { id: "b", name: "B", status: "stopped", path: "/b", lastRunAt: iso(0) },
    { id: "c", name: "C", status: "stopped", path: "/c", lastRunAt: iso(6 * 60 * 60_000) },
  ];
  assert.deepEqual(lastSessionCluster(projects), ["a", "b"]);
});

test("all timestamps missing -> empty", () => {
  const projects = [
    { id: "a", name: "A", status: "stopped", path: "/a" },
    { id: "b", name: "B", status: "stopped", path: "/b" },
  ];
  assert.deepEqual(lastSessionCluster(projects), []);
});

test("one project only -> that one", () => {
  const projects = [{ id: "a", name: "A", status: "stopped", path: "/a", lastRunAt: iso(0) }];
  assert.deepEqual(lastSessionCluster(projects), ["a"]);
});

test("a malformed timestamp is excluded, no throw", () => {
  const projects = [
    { id: "a", name: "A", status: "stopped", path: "/a", lastRunAt: "not-a-date" },
    { id: "b", name: "B", status: "stopped", path: "/b", lastRunAt: iso(0) },
  ];
  assert.deepEqual(lastSessionCluster(projects), ["b"]);
});

test("empty input -> empty output", () => {
  assert.deepEqual(lastSessionCluster([]), []);
});
