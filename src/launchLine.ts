/**
 * Pure launch-line derivation — SPEC.md §11 "Launch line" (plan 060).
 *
 * ZERO IMPORTS — same requirement as `src/session.ts` and `src/dragGeometry.ts`: `src/store.ts`
 * imports `./api`, which imports `@tauri-apps/api`, which `node --test` cannot resolve under this
 * project's `moduleResolution: "bundler"`. Everything below takes plain data, so this leaf stays
 * reachable without a transpiler (see `launchLine.test.mjs`).
 *
 * # The two rules this file encodes
 *
 * **1. Silent when there is nothing to report.** `launchLine(...)` returns an empty result for a
 * library where every project is settled, in sync and clean, and the component renders nothing at
 * all — no empty state, no "all clear" badge, zero pixels. That is the whole reason this element is
 * allowed to exist outside the fixed card element list.
 *
 * **2. A check that could not run is never silence.** A `"unavailable"` row is not "clean": it is
 * counted into `notChecked` and stated on the line. Rule 1 must never swallow rule 2 — a check that
 * failed to run, rendered identically to "checked, nothing to report", is exactly the bug the
 * previous feature shipped.
 *
 * **"Behind" appears nowhere.** Hangar does not fetch, so it cannot know the remote moved. There is
 * no input to this file capable of carrying a behind count.
 */

/** Structural subset of `VcsStatus` — no import, so this module stays leaf-only. */
export interface LaunchVcs {
  projectId: string;
  /** `"not-a-repo" | "checked" | "unavailable"` — read as an open string so this leaf needs no import. */
  state: string;
  ahead?: number;
  uncommitted?: number;
  detail?: string;
}

/** Structural subset of `ProjectView` this file needs. */
export interface LaunchProject {
  id: string;
  name: string;
  status: string;
}

/** One clickable clause. `detail` is the text after the name, e.g. `30 unpushed`. */
export interface LaunchItem {
  projectId: string;
  name: string;
  detail: string;
  /** Hover text — the full clause plus, for a crash, nothing more. Never repository content. */
  title: string;
}

export interface LaunchLineResult {
  /** In `projects` array order — §11's rule for the grid, applied here for the same reason. */
  items: LaunchItem[];
  /** Names of the projects whose check could not run. Stated as a count, never hidden. */
  notChecked: string[];
}

/** §11: at most three items inline; the rest become a `+N`. */
export const MAX_ITEMS = 3;

/**
 * The line's whole content, derived from data the app already has.
 *
 * `vcs` is `null` until the first snapshot resolves, and a project may be absent from a snapshot
 * taken before it was added. Both mean "not yet looked", which is NOT the same as "looked and
 * failed": neither contributes to `notChecked`, because a line that flashed "3 not checked" for the
 * 200 ms before its own fetch landed would train the user to ignore it. `"unavailable"` — Hangar
 * looked and git did not answer — is the case that gets stated.
 *
 * Order is `projects` array order throughout, never severity. §11 never re-sorts the grid, the
 * Ports panel or the Doctor panel, and this line is read alongside all three.
 */
export function launchLine(
  projects: readonly LaunchProject[],
  vcs: readonly LaunchVcs[] | null,
): LaunchLineResult {
  const byId = new Map<string, LaunchVcs>();
  for (const row of vcs ?? []) byId.set(row.projectId, row);

  const items: LaunchItem[] = [];
  const notChecked: string[] = [];

  for (const project of projects) {
    const row = byId.get(project.id);
    if (row?.state === "unavailable") {
      notChecked.push(project.name);
      // Deliberately falls through: a project whose git check failed can still be `crashed`, and
      // that fact came from somewhere else entirely.
    }

    const clauses: string[] = [];
    // 1. Crashed — already in the data model (§6); surfaced here so one line answers "what needs
    //    me". `stop-failed` is left out on purpose: it is a live state whose only remedy is the
    //    card's own retry Stop button, and §11 already forces its folder open for that reason.
    if (project.status === "crashed") clauses.push("crashed last run");
    // 2. Unpushed — exact, and the case this whole element exists for.
    if (row?.state === "checked" && typeof row.ahead === "number" && row.ahead > 0) {
      clauses.push(`${row.ahead} unpushed`);
    }
    // 3. Uncommitted — a count of paths, never a name and never a diff.
    if (row?.state === "checked" && typeof row.uncommitted === "number" && row.uncommitted > 0) {
      clauses.push(`${row.uncommitted} uncommitted`);
    }

    if (clauses.length === 0) continue;
    const detail = clauses.join(", ");
    items.push({
      projectId: project.id,
      name: project.name,
      detail,
      title: `${project.name} — ${detail}`,
    });
  }

  return { items, notChecked };
}

/** Whether the line renders at all. Silent when clean; never silent when a check could not run. */
export function hasSomethingToSay(result: LaunchLineResult): boolean {
  return result.items.length > 0 || result.notChecked.length > 0;
}
