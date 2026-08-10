/**
 * Pure session-derivation logic — SPEC.md §11 "Resume last session" / "Workspace strip" (plan 051).
 *
 * ZERO IMPORTS — same requirement as `src/dragGeometry.ts`: `src/store.ts` imports `./api`, which
 * imports `@tauri-apps/api`, which `node --test` cannot resolve under this project's
 * `moduleResolution: "bundler"`. Both exports below take plain data, not `ProjectView`, so this
 * leaf stays reachable without a transpiler (see `session.test.mjs`).
 */

/** Structural subset of `ProjectStack` — no import, so this module stays leaf-only. */
export interface SessionStack {
  framework?: string;
  libraries: string[];
}

/** Structural subset of `ProjectView` these two functions need. */
export interface SessionProject {
  id: string;
  name: string;
  status: string;
  lastRunAt?: string;
  path: string;
  stack?: SessionStack;
}

/**
 * §11 "Resume last session": ids of projects whose `lastRunAt` is within `windowMs` of the most
 * recent `lastRunAt`, in `projects` order. Missing or unparseable timestamps are excluded, never
 * thrown on. Default window: 30 minutes — see the plan's maintenance note if it ever needs tuning.
 */
export function lastSessionCluster(
  projects: readonly SessionProject[],
  windowMs = 30 * 60_000,
): string[] {
  const parsed = projects
    .map((p) => ({ id: p.id, at: p.lastRunAt ? Date.parse(p.lastRunAt) : NaN }))
    .filter((p) => !Number.isNaN(p.at));
  if (parsed.length === 0) return [];
  const latest = Math.max(...parsed.map((p) => p.at));
  const within = new Set(parsed.filter((p) => latest - p.at <= windowMs).map((p) => p.id));
  return projects.filter((p) => within.has(p.id)).map((p) => p.id);
}
