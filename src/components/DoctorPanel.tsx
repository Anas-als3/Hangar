/**
 * SPEC.md §11 — the Doctor slide-over (added 2026-08-11, plan 057): one section per registered
 * project, in the same array order `get_preflight` returns, listing what Hangar can already tell
 * before a Run about whether that project would even start. Modelled on `PortsPanel.tsx`'s shell —
 * read that first.
 *
 * **Read-only, with no exceptions.** This panel has exactly three controls: Refresh, Close, and
 * Esc. There is no Fix, no Install, no "create the file for me", and no link that writes — §11 is
 * explicit that it reads and reports, and every remedy stays the user's own action in their own
 * editor. That is a structural property, not a styling one: nothing here imports an action from
 * the store, so a future "just one button" cannot be added without also adding the import.
 *
 * **Findings are text.** Every one is rendered as a JSX text node — never raw-HTML injection,
 * never a template that could interpret markup — because the strings contain names read from files
 * in the user's project. (Plan 057 step 5 greps this file for that API by name; it is deliberately
 * not spelled out here, so the grep stays a real check.)
 */
import { useEffect } from "react";
import { closeDoctor, refreshPreflight, useHangarStore } from "../store";
import type { PreflightFinding, PreflightSeverity } from "../types";

/** §11's three severities. Functional colour only, from the existing §11 status tokens — no new
 *  palette entry, and deliberately no green: a clean project shows a quiet line, never a badge. */
const SEVERITY_TONE: Record<PreflightSeverity, string> = {
  blocker: "text-status-danger",
  warning: "text-text",
  note: "text-muted",
};

const SEVERITY_LABEL: Record<PreflightSeverity, string> = {
  blocker: "Blocker",
  warning: "Warning",
  note: "Note",
};

function FindingRow({ finding }: { finding: PreflightFinding }) {
  return (
    <li className="flex items-start gap-3">
      <span
        className={`mt-px w-16 shrink-0 text-xs font-medium ${SEVERITY_TONE[finding.severity]}`}
      >
        {SEVERITY_LABEL[finding.severity]}
      </span>
      <span className="min-w-0">
        <span className="text-sm text-text">{finding.message}</span>{" "}
        <span className="font-mono text-xs text-muted" title={finding.file}>
          {finding.file}
        </span>
      </span>
    </li>
  );
}

export function DoctorPanel() {
  const { doctorOpen, preflight, preflightPending, projects } = useHangarStore();

  // §11: Esc closes the slide-over — the only keyboard shortcut in v0. No layered confirm to
  // dismiss first, because this panel opens none.
  useEffect(() => {
    if (!doctorOpen) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      closeDoctor();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [doctorOpen]);

  if (!doctorOpen) return null;

  // Every section shares one `checkedAt` (one snapshot per `get_preflight` call) — §11: "the
  // header states when the snapshot was taken".
  const checkedAt = preflight?.[0]?.checkedAt;
  const asOf = checkedAt ? new Date(checkedAt).toLocaleTimeString([], { hour12: false }) : "—";

  return (
    <div className="fixed inset-0 z-20 flex justify-end" role="presentation">
      {/* Click-away closes, same as Esc. §11 enter transition: backdrop fades in. */}
      <div className="hangar-fade-in flex-1 bg-black/40" onClick={closeDoctor} aria-hidden="true" />

      <aside
        role="dialog"
        aria-modal="true"
        aria-label="Doctor"
        className="hangar-slide-in flex h-full w-[min(34rem,92vw)] flex-col border-l border-white/10 bg-surface shadow-2xl"
      >
        <header className="flex items-center justify-between gap-3 border-b border-white/5 px-5 py-4">
          <div className="min-w-0">
            <h2 className="font-display text-base font-medium text-text">Doctor</h2>
            {/* §11: "the header states when the snapshot was taken." While a read is in flight the
                honest header is that it is being taken, not a stale timestamp presented as now. */}
            <p className="mt-0.5 font-mono text-xs text-muted">
              {preflightPending ? "checking…" : `as of ${asOf}`}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              disabled={preflightPending}
              onClick={() => void refreshPreflight()}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-muted"
            >
              Refresh
            </button>
            <button
              type="button"
              aria-label="Close doctor"
              onClick={closeDoctor}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              <span aria-hidden="true">✕</span>
            </button>
          </div>
        </header>

        {/* One section per project, in the array order `get_preflight` returned — never sorted by
            severity (§11, same reasoning as the grid never re-sorting). */}
        <ul className="flex-1 overflow-y-auto">
          {/* "Not fetched yet" and "fetched, and there are none" are different facts. Conflating
              them rendered "No registered projects." at every open — a false statement shown for
              as long as the call took, to a user who does have projects registered. `preflight` is
              kept on screen during a Refresh (same "don't blank a working view" rule as Ports),
              so only the very first fetch shows the bare loading line. */}
          {preflight === null ? (
            <li className="px-5 py-4 text-sm text-muted">
              {preflightPending ? "Checking…" : "Nothing checked yet."}
            </li>
          ) : preflight.length === 0 ? (
            <li className="px-5 py-4 text-sm text-muted">No registered projects.</li>
          ) : (
            preflight.map((report) => {
              const project = projects.find((p) => p.id === report.projectId);
              return (
                <li
                  key={report.projectId}
                  className="border-b border-white/5 px-5 py-3 last:border-b-0"
                >
                  <p className="truncate text-sm text-text">
                    {project?.name ?? report.projectId}
                  </p>
                  {report.findings.length === 0 ? (
                    // §11 "silent when clean": one quiet line. No badge, no score, no percentage.
                    <p className="mt-1.5 text-xs text-muted">Nothing to report.</p>
                  ) : (
                    <ul className="mt-1.5 space-y-1.5">
                      {report.findings.map((finding) => (
                        <FindingRow key={finding.id} finding={finding} />
                      ))}
                    </ul>
                  )}
                </li>
              );
            })
          )}
        </ul>

        {/* §11: the panel carries no control that changes anything. Saying so where the user can
            see it is the point — a diagnostic they trust is worth more than one they fear. */}
        <footer className="border-t border-white/5 px-5 py-3 text-xs text-muted">
          Hangar only reads here — it changes nothing, and never blocks Run. Values in .env are
          never read: only key names.
        </footer>
      </aside>
    </div>
  );
}

export default DoctorPanel;
