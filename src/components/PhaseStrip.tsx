/**
 * SPEC.md §11 signature element — Pull → Install → Start → Ready, lit in amber as each real
 * phase completes. Skipped phases (not a git repo, no install needed) render dimmed, distinct
 * from both lit and not-yet-reached (pending). Built by plan 006 (M6).
 */
import { useHangarStore } from "../store";
import type { ProjectView, Status } from "../types";

type PhaseKey = "updating" | "installing" | "starting" | "running";

const PHASES: ReadonlyArray<{ key: PhaseKey; label: string }> = [
  { key: "updating", label: "Pull" },
  { key: "installing", label: "Install" },
  { key: "starting", label: "Start" },
  { key: "running", label: "Ready" },
];

/** Visible for the whole run, including crashed/stop-failed (shows where it died). Hidden only
 *  at the quiet, never-run/finished `stopped` state — matches "appears when Run is clicked". */
const VISIBLE: ReadonlySet<Status> = new Set<Status>([
  "updating",
  "installing",
  "starting",
  "running",
  "stopping",
  "crashed",
  "stop-failed",
]);

export function PhaseStrip({ project }: { project: ProjectView }) {
  // "Seen" is the phases actually observed via status-changed this run (store.ts's
  // `phasesSeen`, plan 027) — sourced from the store, not component state, so a
  // search-triggered unmount/remount of this card cannot blank the strip.
  const { phasesSeen } = useHangarStore();
  const seen = new Set<string>(phasesSeen[project.id] ?? []);

  if (!VISIBLE.has(project.status)) return null;

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
