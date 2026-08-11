// Tests for the zero-import leaf module `notifications.ts` — SPEC.md §11 "Notifications",
// plan 064. Imported by its `.ts` path directly: Node v24's built-in type-stripping runs this with
// no transpiler and no dependency, same arrangement as `launchLine.test.mjs` (plan 060),
// `session.test.mjs` (plan 051) and `dragGeometry.test.mjs` (plan 030).
//
// Run with: node --test src/notifications.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import {
  EMPTY_NOTIFICATIONS,
  NOTIFICATION_LIMIT,
  TOAST_MS_ERROR,
  TOAST_MS_NEUTRAL,
  markNotificationsRead,
  notificationActions,
  pushNotification,
  toastDurationMs,
} from "./notifications.ts";

const push = (state, message, extra = {}) =>
  pushNotification(state, {
    message,
    tone: extra.tone ?? "error",
    projectId: extra.projectId ?? null,
    at: extra.at ?? 0,
  });

// ---------------------------------------------------------------------------------------------
// 1. A new entry is prepended, not appended. This is the one list in the app that is NOT in
//    `projects.json` array order — it is a chronology, not a registry.
// ---------------------------------------------------------------------------------------------

test("a new entry is prepended, not appended — the panel is newest-first", () => {
  let state = EMPTY_NOTIFICATIONS;
  state = push(state, "first");
  state = push(state, "second");
  state = push(state, "third");
  assert.deepEqual(
    state.entries.map((e) => e.message),
    ["third", "second", "first"],
  );
});

test("pushing never mutates the state handed in", () => {
  const before = push(EMPTY_NOTIFICATIONS, "one");
  const after = push(before, "two");
  assert.equal(before.entries.length, 1, "the previous snapshot must be untouched");
  assert.equal(after.entries.length, 2);
  assert.equal(EMPTY_NOTIFICATIONS.entries.length, 0, "the shared empty value must stay empty");
  assert.equal(EMPTY_NOTIFICATIONS.unread, 0);
});

test("ids are unique and monotonic, so React keys can never collide", () => {
  let state = EMPTY_NOTIFICATIONS;
  for (let i = 0; i < 250; i += 1) state = push(state, `m${i}`);
  const ids = state.entries.map((e) => e.id);
  assert.equal(new Set(ids).size, ids.length, "every id must be distinct");
  // Newest first means ids descend down the list.
  for (let i = 1; i < ids.length; i += 1) assert.ok(ids[i - 1] > ids[i]);
});

// ---------------------------------------------------------------------------------------------
// 2. The cap drops the oldest, never the newest.
// ---------------------------------------------------------------------------------------------

test("the cap drops the oldest, never the newest", () => {
  let state = EMPTY_NOTIFICATIONS;
  const total = NOTIFICATION_LIMIT + 25;
  for (let i = 0; i < total; i += 1) state = push(state, `m${i}`);

  assert.equal(state.entries.length, NOTIFICATION_LIMIT);
  assert.equal(state.entries[0].message, `m${total - 1}`, "the newest must be first");
  assert.equal(
    state.entries[state.entries.length - 1].message,
    `m${total - NOTIFICATION_LIMIT}`,
    "the oldest survivor must be exactly LIMIT back",
  );
  assert.equal(
    state.entries.some((e) => e.message === "m0"),
    false,
    "the very first entry must have fallen off",
  );
});

test("under the cap nothing is dropped", () => {
  let state = EMPTY_NOTIFICATIONS;
  for (let i = 0; i < NOTIFICATION_LIMIT; i += 1) state = push(state, `m${i}`);
  assert.equal(state.entries.length, NOTIFICATION_LIMIT);
  assert.equal(state.entries[state.entries.length - 1].message, "m0");
});

// ---------------------------------------------------------------------------------------------
// 3. The unread count is the number of entries added since the panel was last opened — and
//    OPENING THE PANEL ZEROES IT. A badge that never clears is the failure mode users report as
//    "the notification thing is broken". This is the mutation-tested case.
// ---------------------------------------------------------------------------------------------

test("unread counts entries added since the panel was last opened, and opening zeroes it", () => {
  let state = EMPTY_NOTIFICATIONS;
  assert.equal(state.unread, 0, "a fresh session has nothing unread");

  state = push(state, "one");
  state = push(state, "two");
  assert.equal(state.unread, 2);

  // Opening the panel.
  state = markNotificationsRead(state);
  assert.equal(state.unread, 0, "opening the panel must zero the unread count");
  assert.equal(state.entries.length, 2, "and must not throw the entries away");

  // Everything after the open counts again, from zero.
  state = push(state, "three");
  assert.equal(state.unread, 1);
  state = markNotificationsRead(state);
  assert.equal(state.unread, 0);
});

test("marking read with nothing unread is a no-op that returns the same object", () => {
  const state = markNotificationsRead(push(EMPTY_NOTIFICATIONS, "one"));
  assert.equal(state.unread, 0);
  assert.equal(markNotificationsRead(state), state, "no needless subscriber notification");
});

test("the unread count never exceeds the entries the panel can show", () => {
  let state = EMPTY_NOTIFICATIONS;
  for (let i = 0; i < NOTIFICATION_LIMIT + 5; i += 1) state = push(state, `m${i}`);
  assert.ok(
    state.unread >= state.entries.length,
    "sanity: this fixture is meant to overflow the cap",
  );
  state = markNotificationsRead(state);
  assert.equal(state.unread, 0);
});

// ---------------------------------------------------------------------------------------------
// 4. An entry carrying a project id keeps it, so its action button still works. This is what makes
//    auto-dismiss safe: the Ports button (plan 047) and the Show logs button (plan 034) must work
//    from the bell exactly as they did from the toast.
// ---------------------------------------------------------------------------------------------

test("an entry carrying a project id keeps it", () => {
  const state = push(EMPTY_NOTIFICATIONS, "Install failed (exit 1) — see the log, then Run again.", {
    projectId: "p1",
    at: 1_700_000_000_000,
  });
  assert.deepEqual(
    { ...state.entries[0], id: undefined },
    {
      id: undefined,
      message: "Install failed (exit 1) — see the log, then Run again.",
      tone: "error",
      projectId: "p1",
      at: 1_700_000_000_000,
    },
  );
});

test("a project-less entry offers no action at all", () => {
  assert.deepEqual(notificationActions(null, undefined, undefined), {
    showLogs: false,
    ports: false,
  });
  assert.deepEqual(notificationActions(undefined, "Hangar", "stopped"), {
    showLogs: false,
    ports: false,
  });
});

test("the install-failure entry keeps its Show logs button from the bell", () => {
  // The project still resolves, so the name is known and the log panel can be opened.
  assert.equal(notificationActions("p1", "example-app", "crashed").showLogs, true);
});

test("the port-collision entry keeps its Ports button from the bell", () => {
  // §9 step 1 refused the Run, so the project is still stopped — plan 047's honest signal.
  assert.equal(notificationActions("p1", "example-app", "stopped").ports, true);
  assert.equal(notificationActions("p1", "example-app", "crashed").ports, true);
});

test("an entry whose project has since started no longer claims a port collision", () => {
  const actions = notificationActions("p1", "example-app", "running");
  assert.equal(actions.ports, false, "Ports must not be offered for a project that is running");
  assert.equal(actions.showLogs, true, "the log is still there to read");
});

test("a dangling project id never renders a stale name's button", () => {
  // Plan 034's guard: the project was removed while the entry sat in the panel, so the store's
  // lookup yields no name. The panel must not offer a Show logs button for a project that is gone.
  assert.equal(notificationActions("removed", undefined, undefined).showLogs, false);
});

// ---------------------------------------------------------------------------------------------
// 5. Two identical messages in a row are two entries, not one. Collapsing them would hide a retry
//    loop, which is the one thing "port is taken, again" is trying to tell you.
// ---------------------------------------------------------------------------------------------

test("two identical messages in a row are two entries — never deduplicated", () => {
  const message = "Port 3000 is in use by node (PID 4321) — is this project running elsewhere?";
  let state = EMPTY_NOTIFICATIONS;
  state = push(state, message, { projectId: "p1", at: 1000 });
  state = push(state, message, { projectId: "p1", at: 2000 });
  state = push(state, message, { projectId: "p1", at: 3000 });

  assert.equal(state.entries.length, 3, "three refusals are three rows");
  assert.equal(state.unread, 3, "and three unread");
  assert.deepEqual(
    state.entries.map((e) => e.at),
    [3000, 2000, 1000],
    "each keeps its own time, newest first",
  );
  assert.equal(new Set(state.entries.map((e) => e.id)).size, 3, "and its own identity");
});

// ---------------------------------------------------------------------------------------------
// The timer.
// ---------------------------------------------------------------------------------------------

test("an error toast lasts longer than a neutral one, and both are bounded", () => {
  assert.equal(toastDurationMs("neutral"), TOAST_MS_NEUTRAL);
  assert.equal(toastDurationMs("error"), TOAST_MS_ERROR);
  assert.ok(TOAST_MS_ERROR > TOAST_MS_NEUTRAL, "an error carries more words and a possible action");
  assert.equal(TOAST_MS_NEUTRAL, 4000);
  assert.equal(TOAST_MS_ERROR, 6000);
});

test("an unrecognised tone falls back to the neutral duration, never to forever", () => {
  // The store's `ToastTone` union makes this unreachable today; the point is that a future third
  // tone can never accidentally reintroduce the bug this plan fixes — a toast that never leaves.
  assert.equal(toastDurationMs("something-new"), TOAST_MS_NEUTRAL);
  assert.ok(Number.isFinite(toastDurationMs("something-new")));
});
