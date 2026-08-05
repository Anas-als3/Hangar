/**
 * One project card — SPEC.md §11.
 *
 * M2 wired the primary **Run** button and **Show logs**; M3 wires **Stop** — live in every active
 * phase, a disabled spinner while `stopping`, and a retry from `stop-failed`. The other menu
 * entries land with plan 005 (dialogs), the phase strip and the uptime slot with plan 006.
 */
import { useEffect, useRef, useState } from "react";
import { openLogs, startProject, stopProjectAction } from "../store";
import type { ProjectView, Status } from "../types";

const STATUS_LABEL: Record<Status, string> = {
  stopped: "Stopped",
  updating: "Updating",
  installing: "Installing",
  starting: "Starting",
  running: "Running",
  stopping: "Stopping",
  crashed: "Crashed",
  "stop-failed": "Stop failed",
};

/** §11: status colors are functional only. Tokens come from `src/index.css` — no raw hex here. */
const STATUS_TONE: Record<Status, string> = {
  stopped: "text-status-stopped",
  updating: "text-status-active hangar-pulse",
  installing: "text-status-active hangar-pulse",
  starting: "text-status-active hangar-pulse",
  running: "text-status-running",
  stopping: "text-status-active hangar-pulse",
  crashed: "text-status-danger",
  "stop-failed": "text-status-danger",
};

const ACTIVE_STATUSES: ReadonlySet<Status> = new Set<Status>([
  "updating",
  "installing",
  "starting",
  "running",
  "stopping",
]);

/** Relative last-run time. Coarse on purpose — §11 forbids ticking seconds. */
function lastRunLabel(iso: string | undefined): string {
  if (!iso) return "Never run";
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "Never run";
  const minutes = Math.floor((Date.now() - then) / 60_000);
  if (minutes < 1) return "Last run just now";
  if (minutes < 60) return `Last run ${minutes} m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Last run ${hours} h ago`;
  const days = Math.floor(hours / 24);
  return `Last run ${days} d ago`;
}

/** §11 overflow menu. Only "Show logs" is live at M2 — the rest arrive with plans 004/005. */
const MENU_ITEMS = [
  "Open in browser",
  "Open in editor",
  "Show logs",
  "Edit",
  "Remove",
] as const;

const LIVE_MENU_ITEMS: ReadonlySet<(typeof MENU_ITEMS)[number]> = new Set(["Show logs"]);

export function ProjectCard({ project }: { project: ProjectView }) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!menuOpen) return;
    function onPointerDown(event: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setMenuOpen(false);
    }
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [menuOpen]);

  // §11: a crashed card's primary button is Run (retry). While `stopping` it is a disabled
  // spinner. A `stop-failed` card keeps Stop, enabled, so the user can retry the kill (§6/§12).
  const primaryIsStop =
    ACTIVE_STATUSES.has(project.status) || project.status === "stop-failed";
  const stopping = project.status === "stopping";
  const stopFailed = project.status === "stop-failed";
  const runDisabled = !project.pathExists;

  return (
    <article className="relative flex flex-col gap-4 rounded-lg border border-white/5 bg-surface p-5 transition-transform duration-150 hover:-translate-y-0.5">
      <header className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="truncate font-display text-lg font-medium text-text">
            {project.name}
          </h2>
          <p className="mt-1 truncate font-mono text-xs text-muted" title={project.path}>
            {project.path}
          </p>
        </div>

        <div className="relative" ref={menuRef}>
          <button
            type="button"
            aria-label={`Actions for ${project.name}`}
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            onClick={() => setMenuOpen((open) => !open)}
            className="rounded px-2 py-1 text-muted transition-colors hover:bg-white/5 hover:text-text"
          >
            <span aria-hidden="true">⋯</span>
          </button>
          {menuOpen && (
            <div
              role="menu"
              className="absolute right-0 z-10 mt-1 w-44 overflow-hidden rounded-md border border-white/10 bg-bg py-1 shadow-lg"
            >
              {MENU_ITEMS.map((item) => {
                const live = LIVE_MENU_ITEMS.has(item);
                return (
                  <button
                    key={item}
                    role="menuitem"
                    type="button"
                    disabled={!live}
                    onClick={
                      live
                        ? () => {
                            setMenuOpen(false);
                            void openLogs(project.id);
                          }
                        : undefined
                    }
                    className={
                      live
                        ? "block w-full px-3 py-1.5 text-left text-sm text-text transition-colors hover:bg-white/5"
                        : "block w-full cursor-not-allowed px-3 py-1.5 text-left text-sm text-muted opacity-50"
                    }
                  >
                    {item}
                  </button>
                );
              })}
            </div>
          )}
        </div>
      </header>

      <div className="flex items-center gap-2 text-sm">
        <span
          className={`inline-flex items-center gap-2 rounded-full bg-white/5 px-2.5 py-1 ${STATUS_TONE[project.status]}`}
        >
          <span aria-hidden="true" className="size-1.5 rounded-full bg-current" />
          <span>{STATUS_LABEL[project.status]}</span>
          <span className="font-mono text-xs opacity-80">:{project.port}</span>
        </span>

        {!project.pathExists && (
          <span className="rounded-full bg-status-danger/10 px-2.5 py-1 text-xs text-status-danger">
            Folder not found
          </span>
        )}
      </div>

      {/* §11 time slot — uptime while running, otherwise last-run relative time.
          M1 has no running projects yet; plan 002 supplies the uptime branch. */}
      <p className="text-xs text-muted">{lastRunLabel(project.lastRunAt)}</p>

      <footer className="mt-auto flex items-center justify-between gap-3">
        {primaryIsStop ? (
          <button
            type="button"
            disabled={stopping}
            onClick={() => void stopProjectAction(project.id)}
            title={
              stopFailed
                ? "The last Stop could not be verified — press Stop to try again"
                : undefined
            }
            className={`inline-flex items-center gap-2 rounded-md border px-4 py-2 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${
              stopFailed
                ? "border-status-danger/50 text-status-danger hover:bg-status-danger/10"
                : "border-white/10 text-text hover:bg-white/5"
            }`}
          >
            {stopping && (
              <span
                aria-hidden="true"
                className="hangar-spin size-3.5 rounded-full border-2 border-current border-t-transparent"
              />
            )}
            {stopping ? "Stopping…" : "Stop"}
          </button>
        ) : (
          <button
            type="button"
            disabled={runDisabled}
            title={runDisabled ? "The project folder no longer exists" : undefined}
            onClick={() => void startProject(project.id)}
            className="rounded-md bg-accent px-5 py-2 text-sm font-semibold text-bg transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
          >
            Run
          </button>
        )}
        <span className="truncate font-mono text-xs text-muted" title={project.command}>
          {project.command}
        </span>
      </footer>
    </article>
  );
}

export default ProjectCard;
