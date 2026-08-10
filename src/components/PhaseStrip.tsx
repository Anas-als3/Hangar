/**
 * SPEC.md §11 signature element — Pull → Install → Start → Ready, lit in amber as each real
 * phase completes. Skipped phases (not a git repo, no install needed) render dimmed, distinct
 * from both lit and not-yet-reached (pending). Built by plan 006 (M6).
 */
import { useHangarStore } from "../store";
import type { ProjectView } from "../types";

type PhaseKey = "updating" | "installing" | "starting" | "running";

const PHASES: ReadonlyArray<{ key: PhaseKey; label: string }> = [
  { key: "updating", label: "Pull" },
  { key: "installing", label: "Install" },
  { key: "starting", label: "Start" },
  { key: "running", label: "Ready" },
];

export function PhaseStrip({ project }: { project: ProjectView }) {
  // "Seen" is the phases actually observed via status-changed this run (store.ts's
  // `phasesSeen`, plan 027) — sourced from the store, not component state, so a
  // search-triggered unmount/remount of this card cannot blank the strip.
  const { phasesSeen } = useHangarStore();
  // Plan 046 step 5 (§11 amended 2026-08-10): every card carries this strip now, including
  // `stopped` — previously hidden entirely via a `VISIBLE` status set (now removed; it covered
  // exactly the 7 non-`stopped` statuses, so this `=== "stopped"` check is its full replacement).
  // FORCE-UNLIT: `phasesSeen` (store.ts) is cleared only on the transition OUT of
  // stopped/crashed, so running -> stopping -> stopped leaves all four phases in the array. A
  // naive always-on strip would show four accent-lit segments reading "Ready" under a slate
  // "Stopped" pill for any project run earlier this session. Render-time only — no store change.
  const seen =
    project.status === "stopped" ? new Set<string>() : new Set<string>(phasesSeen[project.id] ?? []);

  // Highest phase-index actually observed. Anything earlier that was never seen was skipped.
  const reachedIndex = PHASES.reduce(
    (max, { key }, i) => (seen.has(key) ? Math.max(max, i) : max),
    -1,
  );

  return (
    // Negative margins/padding here must equal ProjectCard's shell padding (p-3) so the
    // strip bleeds exactly to the card's edge — change both together, or the strip will
    // either fall short of the edge or overshoot it into the grid gutter.
    <div
      className="-mx-3 -mb-3 mt-1 flex gap-1 border-t border-white/5 px-3 py-2"
      aria-hidden="true"
    >
      {PHASES.map(({ key, label }, i) => {
        const lit = seen.has(key);
        const dimmed = !lit && i < reachedIndex;
        return (
          <div key={key} className="flex flex-1 flex-col items-center gap-1">
            <span
              className={`h-1 w-full rounded-full transition-colors duration-200 ${
                lit ? "bg-accent" : dimmed ? "bg-status-stopped/50" : "bg-white/10"
              }`}
            />
            <span
              className={`text-[10px] uppercase tracking-wide transition-colors duration-200 ${
                lit ? "text-accent" : dimmed ? "text-muted/70" : "text-muted/30"
              }`}
            >
              {label}
            </span>
          </div>
        );
      })}
    </div>
  );
}

export default PhaseStrip;
