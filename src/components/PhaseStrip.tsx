/**
 * SPEC.md §11 signature element — Pull → Install → Start → Ready, lit in amber as each real
 * phase completes. Skipped phases (not a git repo, no install needed) render dimmed, distinct
 * from both lit and not-yet-reached (pending). Built by plan 006 (M6).
 */
import { useEffect, useRef, useState } from "react";
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

function isPhaseKey(status: Status): status is PhaseKey {
  return (
    status === "updating" ||
    status === "installing" ||
    status === "starting" ||
    status === "running"
  );
}

export function PhaseStrip({ project }: { project: ProjectView }) {
  // "Seen" accumulates the real phases observed via status-changed this run, so a phase that
  // never fired (skipped) can be told apart from one that simply hasn't happened yet.
  const [seen, setSeen] = useState<ReadonlySet<PhaseKey>>(() =>
    isPhaseKey(project.status) ? new Set([project.status]) : new Set(),
  );
  const prevStatus = useRef<Status>(project.status);

  useEffect(() => {
    const previous = prevStatus.current;
    if (previous === project.status) return;
    prevStatus.current = project.status;
    // A fresh Run (leaving stopped/crashed) resets the strip — mirrors store.ts's own
    // definition of "a run is starting" so the two stay in lockstep.
    const freshRun =
      (previous === "stopped" || previous === "crashed") &&
      project.status !== "stopped" &&
      project.status !== "crashed";
    setSeen((current) => {
      const base = freshRun ? new Set<PhaseKey>() : current;
      if (!isPhaseKey(project.status)) return base;
      return new Set(base).add(project.status);
    });
  }, [project.status]);

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
