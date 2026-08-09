/**
 * One project card — SPEC.md §11.
 *
 * M2 wired the primary **Run** button and **Show logs**; M3 wired **Stop** — live in every active
 * phase, a disabled spinner while `stopping`, and a retry from `stop-failed`; M4 wired **Open in
 * browser**; M5 wired **Open in editor**, **Edit** and **Remove**. Plan 006 (M6) adds the §11
 * signature phase strip and the running-uptime time slot.
 */
import { useEffect, useRef, useState } from "react";
import {
  findProject,
  openEditDialog,
  openInBrowserAction,
  openInEditorAction,
  openLogs,
  removeProjectAction,
  startProject,
  stopIfRunningWithConfirm,
  stopProjectAction,
} from "../store";
import type { ProjectView, Status } from "../types";
import { PhaseStrip } from "./PhaseStrip";

/** §6/§10 step 7: Edit — confirm-and-stop first if the project isn't stopped/crashed. */
async function handleEdit(projectId: string): Promise<void> {
  const project = findProject(projectId);
  if (!project) return;
  const okToProceed = await stopIfRunningWithConfirm(project);
  if (!okToProceed) return;
  openEditDialog(project);
}

/** §6/§10 step 7: Remove — same confirm-and-stop, plus a plain destructive-action confirm. */
async function handleRemove(projectId: string): Promise<void> {
  const project = findProject(projectId);
  if (!project) return;
  const okToProceed = await stopIfRunningWithConfirm(project);
  if (!okToProceed) return;
  if (!window.confirm(`Remove ${project.name}? This cannot be undone.`)) return;
  await removeProjectAction(projectId);
}

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

/** §11 time slot while `running`: uptime from the current run's start (`lastRunAt`, set when
 *  entering `starting` — SPEC.md §5/§6), coarse on purpose — no ticking seconds. */
function uptimeLabel(startIso: string | undefined, now: number): string | null {
  if (!startIso) return null;
  const start = Date.parse(startIso);
  if (Number.isNaN(start)) return null;
  const minutes = Math.floor((now - start) / 60_000);
  if (minutes < 1) return "up <1 m";
  if (minutes < 60) return `up ${minutes} m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `up ${hours} h ${minutes % 60} m`;
  return `up ${Math.floor(hours / 24)} d`;
}

/** Refreshes at 30 s granularity while `active`, and re-syncs the instant it becomes active —
 *  never on a per-second tick (§11 explicitly forbids ticking seconds). */
function useCoarseNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(id);
  }, [active]);
  return now;
}

/** §11 overflow menu, in the spec's order. All five entries are wired as of plan 005. */
const MENU_ITEMS: ReadonlyArray<{
  label: string;
  action: ((projectId: string) => void) | null;
}> = [
  { label: "Open in browser", action: (id) => void openInBrowserAction(id) },
  { label: "Open in editor", action: (id) => void openInEditorAction(id) },
  { label: "Show logs", action: (id) => void openLogs(id) },
  { label: "Edit", action: (id) => void handleEdit(id) },
  { label: "Remove", action: (id) => void handleRemove(id) },
];

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
  const isRunning = project.status === "running";
  const now = useCoarseNow(isRunning);
  const timeSlot =
    (isRunning ? uptimeLabel(project.lastRunAt, now) : null) ??
    lastRunLabel(project.lastRunAt);

  return (
    <article className="hangar-fade-in relative flex flex-col gap-3 rounded-lg border border-white/5 bg-surface p-5 transition-transform duration-150 hover:-translate-y-0.5">
      <header className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          {/* §11 visual pass (plan 018): name is the primary hierarchy element — weight only,
              still Space Grotesk / same size, no new element. */}
          <h2 className="truncate font-display text-lg font-bold tracking-tight text-text">
            {project.name}
          </h2>
          <p className="mt-0.5 truncate font-mono text-xs text-muted/80" title={project.path}>
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
              {MENU_ITEMS.map(({ label, action }) => (
                <button
                  key={label}
                  role="menuitem"
                  type="button"
                  disabled={action === null}
                  onClick={
                    action
                      ? () => {
                          setMenuOpen(false);
                          action(project.id);
                        }
                      : undefined
                  }
                  className={
                    action
                      ? "block w-full px-3 py-1.5 text-left text-sm text-text transition-colors hover:bg-white/5"
                      : "block w-full cursor-not-allowed px-3 py-1.5 text-left text-sm text-muted opacity-50"
                  }
                >
                  {label}
                </button>
              ))}
            </div>
          )}
        </div>
      </header>

      {/* §11 visual pass: the status pill is what the user scans a dense grid for — give it
          more presence (padding, weight) without changing its colour meanings. */}
      <div className="flex items-center gap-2 text-sm">
        <span
          className={`inline-flex items-center gap-2 rounded-full bg-white/5 px-3 py-1.5 font-medium transition-colors duration-200 ${STATUS_TONE[project.status]}`}
        >
          <span aria-hidden="true" className="size-1.5 rounded-full bg-current" />
          <span>{STATUS_LABEL[project.status]}</span>
          <span className="font-mono text-xs font-medium">:{project.port}</span>
        </span>

        {!project.pathExists && (
          <span className="rounded-full bg-status-danger/10 px-2.5 py-1 text-xs text-status-danger">
            Folder not found
          </span>
        )}
      </div>

      {/* §11 time slot — uptime while running (30 s granularity, no ticking seconds),
          otherwise last-run relative time. */}
      <p className="text-xs text-muted">{timeSlot}</p>

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

      <PhaseStrip project={project} />
    </article>
  );
}

export default ProjectCard;
