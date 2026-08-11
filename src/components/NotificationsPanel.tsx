/**
 * SPEC.md §11 — the Notifications slide-over (added 2026-08-11, plan 064): every toast raised this
 * session, newest first. Modelled on `DoctorPanel.tsx`'s shell, which is modelled on
 * `PortsPanel.tsx`'s — read those first.
 *
 * **This is what makes auto-dismiss safe.** §7 makes toasts the only surface for every command
 * error, and two of them carry an action the user needs: the port-collision toast's **Ports** button
 * (plan 047) and the install-failure toast's **Show logs** button (plan 034). A toast that expired
 * with nowhere to retrieve it would *destroy* information the app used to keep, so the rows below
 * carry those same buttons, derived by the same `notificationActions` the toast itself uses — one
 * predicate, so the two can never disagree.
 *
 * **Newest first.** This is the one list in the app that is not in `projects.json` array order. It
 * is a chronology, not a registry, and §11 says so explicitly so it does not read as an oversight.
 *
 * **It reads local state and nothing else.** There is no command, no fetch, no Refresh button and
 * no timer here — the Ports/Doctor/Inbox "snapshot, not a monitor" rule is satisfied trivially,
 * because there is nothing this panel could poll. Nothing here is written to disk.
 */
import { useEffect } from "react";
import { notificationActions } from "../notifications";
import type { NotificationEntry } from "../notifications";
import { relativeTime } from "../status";
import {
  closeNotifications,
  openLogs,
  openPorts,
  useHangarStore,
} from "../store";
import type { ProjectView } from "../types";

/** Mirrors the toast's own tone treatment in `App.tsx` — a row should look like the toast it came
 *  from. Functional colour only, from the existing §11 tokens; no new palette entry. */
function toneBorder(tone: string): string {
  return tone === "neutral" ? "border-white/10" : "border-status-danger/40";
}

function EntryRow({ entry, project }: { entry: NotificationEntry; project: ProjectView | undefined }) {
  // Derived **live**, exactly as the toast derives it, from the project as it is right now: an
  // entry can easily outlive the project it names (plan 034's dangling-id guard), and one whose
  // project has since started is no longer pointing at a port collision.
  const actions = notificationActions(entry.projectId, project?.name, project?.status);
  // Narrowed once here so the button's closure captures a `string`, never a `string | null` that
  // would need a cast (CLAUDE.md: TS strict, no `any`).
  const projectId = entry.projectId;
  return (
    <li className="border-b border-white/5 px-5 py-3 last:border-b-0">
      <div className={`border-l-2 pl-3 ${toneBorder(entry.tone)}`}>
        <p className="text-sm text-text">
          {project?.name ? <span className="font-medium">{project.name} — </span> : null}
          {entry.message}
        </p>
        <div className="mt-1.5 flex flex-wrap items-center gap-3">
          <span className="font-mono text-xs text-muted">
            {relativeTime(new Date(entry.at).toISOString())}
          </span>
          {actions.showLogs && projectId && (
            <button
              type="button"
              onClick={() => {
                // Close first: two slide-overs open at once would stack, and one Esc would fire
                // two unrelated state changes — the same rule the folder band's Esc guard follows.
                closeNotifications();
                void openLogs(projectId);
              }}
              className="text-xs text-muted underline-offset-2 transition-colors hover:text-text hover:underline"
            >
              Show logs
            </button>
          )}
          {actions.ports && (
            <button
              type="button"
              onClick={() => {
                closeNotifications();
                void openPorts();
              }}
              className="text-xs text-muted underline-offset-2 transition-colors hover:text-text hover:underline"
            >
              Ports
            </button>
          )}
        </div>
      </div>
    </li>
  );
}

export function NotificationsPanel() {
  const { notificationsOpen, notifications, projects } = useHangarStore();

  // §11: Esc closes the slide-over — the only keyboard shortcut in v0. No layered confirm to
  // dismiss first, because this panel opens none. Same shape as `DoctorPanel`'s.
  useEffect(() => {
    if (!notificationsOpen) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      closeNotifications();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [notificationsOpen]);

  if (!notificationsOpen) return null;

  const entries = notifications.entries;

  return (
    <div className="fixed inset-0 z-20 flex justify-end" role="presentation">
      {/* Click-away closes, same as Esc. §11 enter transition: backdrop fades in. */}
      <div
        className="hangar-fade-in flex-1 bg-black/40"
        onClick={closeNotifications}
        aria-hidden="true"
      />

      <aside
        role="dialog"
        aria-modal="true"
        aria-label="Notifications"
        className="hangar-slide-in flex h-full w-[min(34rem,92vw)] flex-col border-l border-white/10 bg-surface shadow-2xl"
      >
        <header className="flex items-center justify-between gap-3 border-b border-white/5 px-5 py-4">
          <div className="min-w-0">
            <h2 className="font-display text-base font-medium text-text">Notifications</h2>
            {/* Stated in the header rather than left to be inferred: this list is a chronology, so
                it runs newest first — the one place in the app that is not in array order. */}
            <p className="mt-0.5 font-mono text-xs text-muted">
              {entries.length === 0
                ? "nothing this session"
                : `${entries.length} this session · newest first`}
            </p>
          </div>
          {/* No Refresh: there is nothing to re-read. This panel has exactly two controls, Close
              and Esc, plus the per-entry actions the toasts themselves already carried. */}
          <button
            type="button"
            aria-label="Close notifications"
            onClick={closeNotifications}
            className="shrink-0 rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
          >
            <span aria-hidden="true">✕</span>
          </button>
        </header>

        {/* Newest first — see the file header. Never grouped, never filtered, and never
            deduplicated: two identical "port is in use" rows are two real events, and collapsing
            them would hide a retry loop. */}
        <ul className="flex-1 overflow-y-auto">
          {entries.length === 0 ? (
            <li className="px-5 py-4 text-sm text-muted">
              Nothing yet. Messages that appear at the bottom of the window are kept here after they
              fade.
            </li>
          ) : (
            entries.map((entry) => (
              <EntryRow
                key={entry.id}
                entry={entry}
                project={projects.find((p) => p.id === entry.projectId)}
              />
            ))
          )}
        </ul>

        {/* Same honesty as the Doctor panel's footer: say what this surface does and does not do,
            where the user can see it. */}
        <footer className="border-t border-white/5 px-5 py-3 text-xs text-muted">
          Kept in memory for this session only — nothing here is written to disk, and quitting
          Hangar clears it.
        </footer>
      </aside>
    </div>
  );
}

export default NotificationsPanel;
