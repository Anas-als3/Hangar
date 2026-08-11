// Tests for the zero-import leaf module `launchLine.ts` — SPEC.md §11 "Launch line", plan 060.
// Imported by its `.ts` path directly: Node v24's built-in type-stripping runs this with no
// transpiler and no dependency, same arrangement as `session.test.mjs` (plan 051) and
// `dragGeometry.test.mjs` (plan 030).
//
// Run with: node --test src/launchLine.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { hasSomethingToSay, launchLine, MAX_ITEMS } from "./launchLine.ts";

const project = (id, name, status = "stopped") => ({ id, name, status });
const checked = (projectId, ahead, uncommitted) => ({
  projectId,
  state: "checked",
  ahead,
  uncommitted,
});

// --------------------------------------------------------------------------------------------
// Rule 1 — silent when there is nothing to report.
// --------------------------------------------------------------------------------------------

test("every project settled, in sync and clean -> the line renders nothing", () => {
  const projects = [project("a", "example-app"), project("b", "Hangar")];
  const vcs = [checked("a", 0, 0), checked("b", 0, 0)];
  const result = launchLine(projects, vcs);
  assert.deepEqual(result, { items: [], notChecked: [] });
  assert.equal(hasSomethingToSay(result), false);
});

test("an empty library renders nothing", () => {
  assert.equal(hasSomethingToSay(launchLine([], [])), false);
  assert.equal(hasSomethingToSay(launchLine([], null)), false);
});

test("a project that is not a git repo says nothing at all", () => {
  const result = launchLine([project("a", "Scratch")], [{ projectId: "a", state: "not-a-repo" }]);
  assert.equal(hasSomethingToSay(result), false);
});

test("no upstream / detached / no commits -> ahead absent -> nothing said", () => {
  // The backend reports `checked` with `ahead` omitted for all three; only `uncommitted` is known.
  const result = launchLine(
    [project("a", "Fresh")],
    [{ projectId: "a", state: "checked", uncommitted: 0 }],
  );
  assert.equal(hasSomethingToSay(result), false);
});

// --------------------------------------------------------------------------------------------
// Rule 2 — a check that could not run is never silence. This is the bug the previous feature
// shipped: an empty result rendering identically to "checked, nothing to report".
// --------------------------------------------------------------------------------------------

test("a check that could not run is NOT rendered as clean", () => {
  const clean = launchLine([project("a", "Hangar")], [checked("a", 0, 0)]);
  const failed = launchLine(
    [project("a", "Hangar")],
    [{ projectId: "a", state: "unavailable", detail: "git status did not answer within 3 s." }],
  );

  assert.equal(hasSomethingToSay(clean), false, "clean must be silent");
  assert.equal(hasSomethingToSay(failed), true, "a failed check must NOT be silent");
  assert.deepEqual(failed.notChecked, ["Hangar"]);
  assert.notDeepEqual(clean, failed);
});

test("a failed check never becomes an item, so it cannot displace a real finding", () => {
  const projects = [
    project("a", "One"),
    project("b", "Two"),
    project("c", "Three"),
    project("d", "example-monorepo"),
  ];
  const vcs = [
    { projectId: "a", state: "unavailable" },
    { projectId: "b", state: "unavailable" },
    { projectId: "c", state: "unavailable" },
    checked("d", 30, 0),
  ];
  const result = launchLine(projects, vcs);
  assert.deepEqual(
    result.items.map((i) => i.detail),
    ["30 unpushed"],
    "the one real finding must still be the first item",
  );
  assert.equal(result.notChecked.length, 3);
});

test("no snapshot yet (null) is 'not yet looked', not 'could not check'", () => {
  // Rendering "3 not checked" for the 200 ms before the first fetch lands would train the user to
  // ignore the line. A crashed project still shows, because that fact does not come from git.
  const projects = [project("a", "One"), project("b", "Two", "crashed")];
  const result = launchLine(projects, null);
  assert.deepEqual(result.notChecked, []);
  assert.deepEqual(
    result.items.map((i) => `${i.name} ${i.detail}`),
    ["Two crashed last run"],
  );
});

test("a project absent from the snapshot is not reported as failed", () => {
  const result = launchLine([project("a", "One"), project("b", "Added just now")], [checked("a", 0, 0)]);
  assert.equal(hasSomethingToSay(result), false);
});

// --------------------------------------------------------------------------------------------
// The measurement this feature exists for.
// --------------------------------------------------------------------------------------------

test("thirty commits that exist only on this laptop are named exactly", () => {
  const result = launchLine([project("a", "example-monorepo")], [checked("a", 30, 0)]);
  assert.deepEqual(result.items, [
    {
      projectId: "a",
      name: "example-monorepo",
      detail: "30 unpushed",
      title: "example-monorepo — 30 unpushed",
    },
  ]);
});

test("one project with several facts is one item, in a fixed clause order", () => {
  const result = launchLine([project("a", "Hangar", "crashed")], [checked("a", 2, 1)]);
  assert.equal(result.items.length, 1);
  assert.equal(result.items[0].detail, "crashed last run, 2 unpushed, 1 uncommitted");
});

test("uncommitted is a count and nothing else", () => {
  const result = launchLine([project("a", "Hangar")], [checked("a", 0, 1)]);
  assert.equal(result.items[0].detail, "1 uncommitted");
});

test("crashed alone renders without any vcs row", () => {
  const result = launchLine([project("a", "Boom", "crashed")], [{ projectId: "a", state: "not-a-repo" }]);
  assert.equal(result.items[0].detail, "crashed last run");
});

test("stop-failed is not a launch-line item — its only remedy is the card's own button", () => {
  const result = launchLine([project("a", "Stuck", "stop-failed")], [checked("a", 0, 0)]);
  assert.equal(hasSomethingToSay(result), false);
});

// --------------------------------------------------------------------------------------------
// Ordering and the cap.
// --------------------------------------------------------------------------------------------

test("items are in projects array order, never sorted by severity", () => {
  const projects = [project("a", "A"), project("b", "B", "crashed"), project("c", "C")];
  const vcs = [checked("a", 1, 0), checked("b", 0, 0), checked("c", 99, 0)];
  assert.deepEqual(
    launchLine(projects, vcs).items.map((i) => i.name),
    ["A", "B", "C"],
  );
});

test("more than three findings -> the caller shows three and a +N", () => {
  const projects = ["a", "b", "c", "d", "e"].map((id) => project(id, id.toUpperCase()));
  const vcs = projects.map((p) => checked(p.id, 1, 0));
  const result = launchLine(projects, vcs);
  assert.equal(result.items.length, 5);
  assert.equal(MAX_ITEMS, 3);
  assert.equal(result.items.length - MAX_ITEMS, 2, "the +N the component renders");
});

// --------------------------------------------------------------------------------------------
// "Behind" must not exist anywhere in the rendered text — Hangar never fetches.
// --------------------------------------------------------------------------------------------

test("nothing this module produces can say 'behind'", () => {
  const projects = [project("a", "example-monorepo", "crashed"), project("b", "Hangar")];
  const vcs = [checked("a", 30, 4), { projectId: "b", state: "unavailable", detail: "git missing" }];
  const result = launchLine(projects, vcs);
  const rendered = JSON.stringify(result);
  assert.match(rendered, /30 unpushed/, "the ahead count must be rendered, or this guard is vacuous");
  assert.doesNotMatch(rendered, /behind/i);
});
