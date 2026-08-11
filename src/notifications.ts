/**
 * Pure notification-history derivation — SPEC.md §11 "Notifications" (plan 064).
 *
 * ZERO IMPORTS — same requirement as `src/launchLine.ts`, `src/session.ts` and
 * `src/dragGeometry.ts`: `src/store.ts` imports `./api`, which imports `@tauri-apps/api`, which
 * `node --test` cannot resolve under this project's `moduleResolution: "bundler"`. Everything below
 * takes plain data, so this leaf stays reachable without a transpiler (see `notifications.test.mjs`).
 *
 * # The rules this file encodes
 *
 * **1. A dismissed toast is not a lost toast.** SPEC.md §7 makes toasts the *only* surface for every
 * command error, and §9/§12 put real, actionable text in them ("Port 3000 is in use by node (PID
 * 4321) …", "Install failed (exit 1) — see the log, then Run again."). Auto-dismiss on its own would
 * therefore *destroy* information the app used to keep. Every toast is pushed here first, and the
 * bell is where it goes — the timer is only safe because this list exists.
 *
 * **2. Newest first.** This is the one list in the app that is not in `projects.json` array order.
 * It is a chronology, not a registry: "what just happened" is the question it answers, so the most
 * recent entry is the first one. That is deliberate, not an oversight — §11 says so in as many
 * words.
 *
 * **3. Never deduplicate.** Two identical "port is in use" messages are two real events. Collapsing
 * them into one row with a ×2 would hide a retry loop, which is exactly the thing the user needs to
 * see. `pushNotification` has no equality check anywhere and must never grow one.
 *
 * **4. Ephemeral.** Nothing here is written to disk. There is no `projects.json` field, no settings
 * key and no new file — §5's data model does not change for a notification log.
 */

/**
 * One entry. `tone` and `status` are read as open strings rather than the `ToastTone`/`Status`
 * unions so this module needs no import; the store passes the real values through unchanged.
 */
export interface NotificationEntry {
  /** Monotonic within a session — see `pushNotification`. Used as the React key, nothing else. */
  id: number;
  message: string;
  /** `"error" | "neutral"` — §11's two toast tones, and there is deliberately no third. */
  tone: string;
  /** Plan 034/047: which project this was about, so the entry keeps its action button. */
  projectId: string | null;
  /** Epoch ms, so the tests need no clock and no ISO parsing. Rendered via `relativeTime`. */
  at: number;
}

/** Entries plus the unread tally. One object so the two can never be updated out of step. */
export interface NotificationState {
  /** **Newest first** — see rule 2 above. */
  entries: NotificationEntry[];
  /** Entries added since the panel was last opened. Zeroed by `markNotificationsRead`. */
  unread: number;
}

export const EMPTY_NOTIFICATIONS: NotificationState = { entries: [], unread: 0 };

/**
 * §11: "capped at a fixed recent count". 100 is ample for a session's worth of run failures, and
 * the whole list is in memory, so the cap is about the panel staying readable, not about memory.
 */
export const NOTIFICATION_LIMIT = 100;

/** What the store hands over — everything but the id, which this module assigns. */
export interface NotificationInput {
  message: string;
  tone: string;
  projectId: string | null;
  at: number;
}

/**
 * Prepend, then drop from the tail.
 *
 * The id is derived from the current newest rather than a module-level counter, so this function
 * stays pure and the tests need no reset hook: `entries[0]` is by construction the highest id, and
 * the cap only ever removes from the *end*, so `entries[0].id + 1` cannot collide with a live entry.
 *
 * **No deduplication** (rule 3). Two identical messages in a row are two entries.
 */
export function pushNotification(
  state: NotificationState,
  input: NotificationInput,
): NotificationState {
  const entry: NotificationEntry = {
    id: state.entries.length === 0 ? 1 : state.entries[0].id + 1,
    message: input.message,
    tone: input.tone,
    projectId: input.projectId,
    at: input.at,
  };
  const entries = [entry, ...state.entries];
  return {
    // `slice` from the front keeps the newest — the oldest fall off, never the entry just added.
    entries: entries.length > NOTIFICATION_LIMIT ? entries.slice(0, NOTIFICATION_LIMIT) : entries,
    unread: state.unread + 1,
  };
}

/**
 * Opening the panel zeroes the unread count — the user has now seen them.
 *
 * Returns the same object when there is nothing to clear, so an idempotent re-open cannot notify
 * `useSyncExternalStore` subscribers for no reason.
 */
export function markNotificationsRead(state: NotificationState): NotificationState {
  if (state.unread === 0) return state;
  return { entries: state.entries, unread: 0 };
}

// ----------------------------------------------------------------------------------------------
// The toast timer
// ----------------------------------------------------------------------------------------------

/** Neutral tone — a confirmation of something the user just did and already knows. */
export const TOAST_MS_NEUTRAL = 4000;

/**
 * Error tone — more words, and often an action to notice before deciding. The maintainer asked for
 * "3~5 seconds"; 6 s for an error is a deliberate, stated overshoot on the same reasoning §11 uses
 * everywhere else: losing an error is worse than the bug being fixed. Hovering pauses it anyway.
 */
export const TOAST_MS_ERROR = 6000;

/** How long this tone's toast stays before it auto-dismisses into the bell. */
export function toastDurationMs(tone: string): number {
  return tone === "error" ? TOAST_MS_ERROR : TOAST_MS_NEUTRAL;
}

// ----------------------------------------------------------------------------------------------
// The action buttons — one derivation, used by both the toast and the panel
// ----------------------------------------------------------------------------------------------

/** Which buttons an entry carries. Both false is the ordinary case. */
export interface NotificationActions {
  /** Plan 034 — "Show logs", opening that project's slide-over. */
  showLogs: boolean;
  /** Plan 047 — "Ports", opening the Ports panel. */
  ports: boolean;
}

/**
 * The single source of truth for whether an entry offers an action, so the toast and the bell panel
 * can never disagree — "the same button, still working, after the toast has gone" is the entire
 * point of plan 064, and two copies of this predicate is exactly how that would quietly stop being
 * true.
 *
 * `projectName` is `undefined` when there is no id **or the id no longer resolves** (project removed
 * meanwhile) — plan 034's dangling-id guard, which must hold in the panel too, where an entry can
 * easily outlive the project it names.
 *
 * The Ports rule is plan 047's, unchanged: `run_project` returns a plain `Result<(), String>` (§7 is
 * frozen), so the frontend cannot tell a port refusal from any other failure by parsing the message
 * — that would be a regex tied to `run.rs`'s wording. The honest signal is that the project is
 * `stopped`/`crashed`, i.e. the Run did not take. Evaluated **live**, from the store's current
 * status, in both places: an entry whose project has since started is no longer pointing at a
 * collision, and offering Ports for it would be a stale claim.
 */
export function notificationActions(
  projectId: string | null | undefined,
  projectName: string | undefined,
  projectStatus: string | undefined,
): NotificationActions {
  if (!projectId) return { showLogs: false, ports: false };
  return {
    showLogs: Boolean(projectName),
    ports: projectStatus === "stopped" || projectStatus === "crashed",
  };
}
