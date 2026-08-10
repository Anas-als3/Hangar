/**
 * One project card — SPEC.md §11.
 *
 * M2 wired the primary **Run** button and **Show logs**; M3 wired **Stop** — live in every active
 * phase, a disabled spinner while `stopping`, and a retry from `stop-failed`; M4 wired **Open in
 * browser**; M5 wired **Open in editor**, **Edit** and **Remove**. Plan 006 (M6) adds the §11
 * signature phase strip and the running-uptime time slot.
 */
import { useEffect, useRef, useState } from "react";
import type { KeyboardEventHandler } from "react";
import { startCardDrag } from "../cardDrag";
import {
  findProject,
  openEditDialog,
  openInBrowserAction,
  openInEditorAction,
  openLogs,
  openMoveToFolderDialog,
  openNotes,
  removeProjectAction,
  startProject,
  stopIfRunningWithConfirm,
  stopProjectAction,
  useHangarStore,
} from "../store";
import { lastRunLabel, relativeTime, STATUS_LABEL, STATUS_TONE } from "../status";
import type { ProjectStack, ProjectView, Status } from "../types";
import { PhaseStrip } from "./PhaseStrip";

/** Plan 035 step 1: one hover string for BOTH the badge and the libraries line, so whichever
 *  renders alone (a Vite project with no allow-listed deps has a badge and no line; a monorepo
 *  root has a line and no badge) still carries the whole stack. Single line, ` · ` separator —
 *  no timestamp, no `\n` (multi-line `title` renders differently across webviews). */
function stackHoverText(stack: ProjectStack): string | null {
  const parts = stack.framework ? [stack.framework, ...stack.libraries] : stack.libraries;
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** §6/§10 step 7: Edit — confirm-and-stop first if the project isn't stopped/crashed. */
async function handleEdit(projectId: string): Promise<void> {
  const project = findProject(projectId);
  if (!project) return;
  const okToProceed = await stopIfRunningWithConfirm(project);
  if (!okToProceed) return;
  openEditDialog(project);
}

/** §11 "Move to folder…" — §5: `folderId`/`folderName` are run-inert, so unlike Edit/Remove this
 *  needs no confirm-and-stop first. */
function handleMoveToFolder(projectId: string): void {
  const project = findProject(projectId);
  if (!project) return;
  openMoveToFolderDialog(project);
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

const ACTIVE_STATUSES: ReadonlySet<Status> = new Set<Status>([
  "updating",
  "installing",
  "starting",
  "running",
  "stopping",
]);

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

/** §11 overflow menu, in the spec's amended order (plan 020 adds Notes after Show logs). */
const MENU_ITEMS: ReadonlyArray<{
  label: string;
  action: ((projectId: string) => void) | null;
}> = [
  { label: "Open in browser", action: (id) => void openInBrowserAction(id) },
  { label: "Open in editor", action: (id) => void openInEditorAction(id) },
  { label: "Show logs", action: (id) => void openLogs(id) },
  { label: "Notes", action: (id) => void openNotes(id) },
  { label: "Move to folder…", action: (id) => handleMoveToFolder(id) },
  { label: "Edit", action: (id) => void handleEdit(id) },
  { label: "Remove", action: (id) => void handleRemove(id) },
];

export function ProjectCard({ project }: { project: ProjectView }) {
  // Plan 037 step 2: one overlay at a time (the `⋯` menu or the stack reveal panel), each with its
  // own ref. The outside-click effect below tests whichever ref matches the open overlay — reusing
  // `menuRef` for the panel would read every click inside the panel as "outside" and close it
  // instantly, since `menuRef` sits on the header div and does not contain the libraries line.
  const [overlay, setOverlay] = useState<"menu" | "stack" | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const stackRef = useRef<HTMLDivElement | null>(null);
  const menuOpen = overlay === "menu";

  useEffect(() => {
    if (!overlay) return;
    const activeRef = overlay === "menu" ? menuRef : stackRef;
    function onPointerDown(event: MouseEvent) {
      if (activeRef.current && !activeRef.current.contains(event.target as Node)) {
        setOverlay(null);
      }
    }
    document.addEventListener("mousedown", onPointerDown);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
    };
  }, [overlay]);

  // Plan 033 defect 1: this used to be a `document` keydown listener, but member cards live
  // inside an open folder's band, which also owns Esc (ProjectGrid.tsx). A document listener
  // can't stop that React handler from also firing, so dismissing this menu closed the whole
  // folder too. Scoping to a React handler on the tree that contains both the trigger and the
  // menu, plus stopPropagation, keeps Esc here from ever reaching the band.
  const onMenuKeyDown: KeyboardEventHandler<HTMLDivElement> = (event) => {
    if (event.key === "Escape" && menuOpen) {
      event.stopPropagation();
      setOverlay(null);
    }
  };

  // Plan 037 step 3: same scoped-Esc shape as `onMenuKeyDown` above — stopPropagation only while
  // the panel is actually open, so a closed panel never eats the folder band's own Esc.
  const stackOpen = overlay === "stack";
  const stackButtonRef = useRef<HTMLButtonElement | null>(null);
  const stackPanelRef = useRef<HTMLDivElement | null>(null);
  const onStackKeyDown: KeyboardEventHandler<HTMLDivElement> = (event) => {
    if (event.key === "Escape" && stackOpen) {
      event.stopPropagation();
      setOverlay(null);
    }
  };

  // Plan 037 step 3.7: focus the panel the instant it opens; when it closes (Esc, outside click,
  // or a second `+N` press), return focus to the `+N` button rather than dropping it to `<body>`.
  const wasStackOpenRef = useRef(false);
  useEffect(() => {
    if (stackOpen) {
      wasStackOpenRef.current = true;
      stackPanelRef.current?.focus();
    } else if (wasStackOpenRef.current) {
      wasStackOpenRef.current = false;
      stackButtonRef.current?.focus();
    }
  }, [stackOpen]);

  // §11: a crashed card's primary button is Run (retry). While `stopping` it is a disabled
  // spinner. A `stop-failed` card keeps Stop, enabled, so the user can retry the kill (§6/§12).
  const primaryIsStop =
    ACTIVE_STATUSES.has(project.status) || project.status === "stop-failed";
  const stopping = project.status === "stopping";
  const stopFailed = project.status === "stop-failed";
  // Plan 052: `pendingRun` is a property of the CLICK, never of the project — it must never
  // render a §6 status name, and the status pill just below is untouched by it (still reads
  // whatever `project.status` says, e.g. "Stopped"). It only ever reaches this one button, only
  // while the button is in its Run shape (`stopped`/`crashed`), so it can never appear on top of
  // the Stop/Stopping/retry branch below. See `pendingRun`'s definition in store.ts for the full
  // reasoning and the two conditions that clear it.
  const { pendingRun, drag } = useHangarStore();
  const isPendingRun = Boolean(pendingRun[project.id]);
  const runDisabled = !project.pathExists || isPendingRun;
  // §4/§5: notes are a free-text scratchpad, never parsed — this only checks for presence.
  const hasNotes = Boolean(project.notes && project.notes.trim() !== "");
  const isRunning = project.status === "running";
  const now = useCoarseNow(isRunning);
  const timeSlot =
    (isRunning ? uptimeLabel(project.lastRunAt, now) : null) ??
    lastRunLabel(project.lastRunAt);

  // Plan 030 drag-to-group (§11 Motion): both effects are opacity/colour only, applied instantly
  // — no transition class on either, so neither can be caught mid-fade by the base transform
  // transition already on this root.
  const isDragSource = drag.sourceId === project.id;
  const isArmedTarget = drag.targetId === project.id && drag.armed;

  return (
    <article
      data-hangar-tile={project.id}
      data-hangar-tile-kind="project"
      onPointerDown={(e) => startCardDrag(e.nativeEvent, project.id)}
      // Plan 046 step 6: p-3 -> p-4, gap-2 -> gap-2.5. PhaseStrip.tsx's negative margins/padding
      // are the same number in a second file — changed together below, see its own comment.
      className={`hangar-fade-in relative flex select-none flex-col gap-2.5 rounded-lg border border-white/5 bg-surface p-4 transition-transform duration-150 [-webkit-user-drag:none] hover:-translate-y-0.5 ${
        isArmedTarget ? "ring-2 ring-accent" : ""
      } ${isDragSource ? "opacity-40" : ""} ${
        // Plan 037 step 3.4 / plan 046 step 1: `hover:-translate-y-0.5` above makes this card its
        // own stacking context, so a `z-10` child stays sealed inside it unless the card itself is
        // lifted above later-DOM-order siblings while an overlay is open — the `⋯` menu (seven
        // items, ~234px, taller than a stopped card) included, not just the stack reveal panel.
        stackOpen || menuOpen ? "z-20" : ""
      }`}
    >
      <header className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          {/* §11 visual pass (plan 018): name is the primary hierarchy element — weight only.
              Plan 046 step 6: text-lg -> text-xl is free height — Tailwind gives both the same
              28px line box, so this costs nothing in the card's fixed silhouette. */}
          <h2
            className="truncate font-display text-xl font-bold tracking-tight text-text"
            title={project.name}
          >
            {project.name}
          </h2>
          <p className="mt-0.5 truncate font-mono text-xs text-muted/80" title={project.path}>
            {project.path}
          </p>
        </div>

        <div className="relative" ref={menuRef} onKeyDown={onMenuKeyDown}>
          <button
            type="button"
            aria-label={
              hasNotes ? `Actions for ${project.name} (has notes)` : `Actions for ${project.name}`
            }
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            onClick={() => setOverlay((open) => (open === "menu" ? null : "menu"))}
            className="relative rounded px-2 py-1 text-muted transition-colors hover:bg-white/5 hover:text-text"
          >
            <span aria-hidden="true">⋯</span>
            {/* §11: a property of the existing control, not a new card element — quiet on purpose. */}
            {hasNotes && (
              <span
                aria-hidden="true"
                className="absolute right-0.5 top-0.5 size-1.5 rounded-full bg-muted"
              />
            )}
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
                          setOverlay(null);
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
          className={`inline-flex items-center gap-2 rounded-full bg-white/5 px-2.5 py-1 font-medium transition-colors duration-200 ${STATUS_TONE[project.status]}`}
        >
          <span aria-hidden="true" className="size-1.5 rounded-full bg-current" />
          <span>{STATUS_LABEL[project.status]}</span>
          {/* §11: the port is the natural click target for the browser — an existing element made
              actionable, not a new one. Inert text when not running: no dead-looking button. */}
          {isRunning ? (
            <button
              type="button"
              onClick={() => void openInBrowserAction(project.id)}
              title={`Open localhost:${project.port} in your browser`}
              aria-label={`Open ${project.name} in your browser on port ${project.port}`}
              className="rounded-sm border-0 bg-transparent p-0 font-mono text-xs font-medium text-current underline-offset-2 hover:underline focus-visible:underline"
            >
              :{project.port}
            </button>
          ) : (
            <span className="font-mono text-xs font-medium">:{project.port}</span>
          )}
        </span>

        {/* §11 (added 2026-08-09): the one permitted stack badge — display-only, derived, never
            a control. Quieter than the status pill on purpose: status is what users scan for.
            `title` (plan 035 step 1) carries the whole stack — this badge can be the only
            stack element a card shows, so it must not depend on the libraries line existing. */}
        {project.stack?.framework && (
          <span
            className="rounded-full border border-white/10 bg-white/5 px-2 py-0.5 font-mono text-xs text-muted"
            title={stackHoverText(project.stack) ?? undefined}
          >
            {project.stack.framework}
          </span>
        )}

        {!project.pathExists && (
          <span className="rounded-full bg-status-danger/10 px-2.5 py-1 text-xs text-status-danger">
            Folder not found
          </span>
        )}
      </div>

      {/* §11 libraries line (added 2026-08-10, `+N` reveal added 2026-08-10 — plan 037): capped,
          display-only text, with the one permitted exception ("the count is the one exception to
          'never controls'"): `+N` is a button revealing the full detected stack in a read-only
          panel. `title` (plan 035 step 1) still carries the whole stack for a plain hover. `<div>`
          + `relative` (not `<p>`) because the panel is `absolute`: a `<ul>` inside a `<p>` is
          invalid nesting the browser would hoist out, and the panel's `w-full` needs a block
          containing block, not the inline `+5` glyph. */}
      {project.stack && project.stack.libraries.length > 0 && (
        <div
          className="relative flex items-baseline gap-1 text-xs text-muted"
          title={stackHoverText(project.stack) ?? undefined}
          ref={stackRef}
          onKeyDown={onStackKeyDown}
        >
          <span className="truncate">{project.stack.libraries.slice(0, 3).join(" · ")}</span>
          {project.stack.libraries.length > 3 && (
            <button
              type="button"
              ref={stackButtonRef}
              aria-label={`+${project.stack.libraries.length - 3} more — show the full stack for ${project.name}`}
              aria-expanded={stackOpen}
              onClick={() => setOverlay((open) => (open === "stack" ? null : "stack"))}
              className="shrink-0 rounded-sm border-0 bg-transparent p-0 text-muted/60 transition-colors hover:text-text"
            >
              +{project.stack.libraries.length - 3}
            </button>
          )}
          {stackOpen && (
            <div
              ref={stackPanelRef}
              tabIndex={-1}
              aria-label={`Detected stack for ${project.name}`}
              onPointerDown={(e) => e.stopPropagation()}
              className="absolute left-0 bottom-full z-10 mb-2 w-full max-h-52 overflow-y-auto select-text rounded-md border border-white/10 bg-bg p-2.5 shadow-lg"
            >
              <ul className="flex flex-wrap gap-1">
                {project.stack.framework && (
                  <li className="rounded-full border border-white/10 bg-white/5 px-2 py-0.5 font-mono text-xs text-muted">
                    {project.stack.framework}
                  </li>
                )}
                {project.stack.libraries.map((lib) => (
                  <li
                    key={lib}
                    className="rounded-full border border-white/10 bg-white/5 px-2 py-0.5 font-mono text-xs text-muted"
                  >
                    {lib}
                  </li>
                ))}
              </ul>
              <p className="mt-2 text-xs text-muted/60">
                detected {relativeTime(project.stack.detectedAt)}
              </p>
            </div>
          )}
        </div>
      )}

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
            title={!project.pathExists ? "The project folder no longer exists" : undefined}
            onClick={() => void startProject(project.id)}
            // Plan 052: copies the `stopping` branch's spinner markup exactly. The status pill
            // above this footer is not part of this branch and is not re-rendered by it — only
            // this button's own label and disabled state move.
            className="inline-flex items-center gap-2 rounded-md bg-accent px-5 py-2 text-sm font-semibold text-bg transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {isPendingRun && (
              <span
                aria-hidden="true"
                className="hangar-spin size-3.5 rounded-full border-2 border-current border-t-transparent"
              />
            )}
            {isPendingRun ? "Starting…" : "Run"}
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
