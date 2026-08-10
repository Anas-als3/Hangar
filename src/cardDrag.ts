/**
 * The pointer-driven card drag session — SPEC.md §11 Motion "drag-to-group feedback", plan 030.
 *
 * A module-level object, not React state: only `{sourceId, targetId, armed}` ever reaches the
 * store (via `setDragView`), because `setState` notifies every subscriber and the grid already
 * re-renders on every log flush — a per-pointermove `setState` would re-render every card dozens
 * of times a second. Pointer coordinates themselves never leave this module.
 */
import { DWELL_MS, hasMovedEnough, hitTest, stillWithinSlop } from "./dragGeometry";
import type { DragTile, Point } from "./dragGeometry";
import { findProject, moveToFolder, setDragView, setToast } from "./store";
import { STATUS_TONE } from "./status";

type TargetKind = "project" | "folder";

interface Session {
  sourceId: string;
  origin: Point;
  moved: boolean;
  ghost: HTMLDivElement | null;
  targetId: string | null;
  targetKind: TargetKind | null;
  targetEl: HTMLElement | null;
  dwellAnchor: Point | null;
  armTimer: ReturnType<typeof setTimeout> | null;
  armed: boolean;
}

let session: Session | null = null;

// Hit-testing runs at most every ~2 frames of pointermove, not on every event — cheap already
// (one `elementFromPoint` + one `getBoundingClientRect`), but no reason to do it 100+ times/sec.
const HITTEST_THROTTLE_MS = 32;
let lastHitTestAt = 0;

const INTERACTIVE_SELECTOR = 'button, a, input, textarea, [role="menu"]';

/**
 * `pointerdown` handler for `ProjectCard`'s root — bail on anything that isn't a plain left click
 * on the card body, or Run, Stop, the `:3000` port link and the `⋯` menu all become drag handles.
 */
export function startCardDrag(event: PointerEvent, projectId: string): void {
  if (event.button !== 0 || event.ctrlKey) return;
  const target = event.target as Element | null;
  if (target?.closest(INTERACTIVE_SELECTOR)) return;
  if (session) return;

  session = {
    sourceId: projectId,
    origin: { x: event.clientX, y: event.clientY },
    moved: false,
    ghost: null,
    targetId: null,
    targetKind: null,
    targetEl: null,
    dwellAnchor: null,
    armTimer: null,
    armed: false,
  };

  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", onPointerCancel);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("blur", onWindowBlur);
}

/**
 * DOM-native hit test — `document.elementFromPoint` over elements carrying `data-hangar-tile`.
 * Never caches rects: a project can crash and vanish mid-drag under an active search, and the DOM
 * is the one source of truth that is always current.
 */
function locateTile(
  clientX: number,
  clientY: number,
): { id: string; kind: TargetKind; el: HTMLElement } | null {
  const hit = document.elementFromPoint(clientX, clientY);
  const el = hit?.closest<HTMLElement>("[data-hangar-tile]") ?? null;
  if (!el) return null;
  const id = el.getAttribute("data-hangar-tile");
  const kind = el.getAttribute("data-hangar-tile-kind");
  if (!id || (kind !== "project" && kind !== "folder")) return null;
  return { id, kind, el };
}

function onPointerMove(event: PointerEvent): void {
  const s = session;
  if (!s) return;
  // §11 reset rule: a `projects` change that removes the source ends the session. Checked here
  // (every move) and again in `arm()` (for the case the pointer never moves during the dwell).
  if (!findProject(s.sourceId)) {
    cancelSession();
    return;
  }
  const point: Point = { x: event.clientX, y: event.clientY };

  if (!s.moved) {
    if (!hasMovedEnough(s.origin, point)) return;
    s.moved = true;
    s.ghost = createGhost(s.sourceId, point);
    if (!s.ghost) {
      cancelSession();
      return;
    }
    setDragView({ sourceId: s.sourceId, targetId: null, armed: false });
  }

  // §11: one style write per pointer event, no rAF loop — the ghost tracks the cursor directly.
  if (s.ghost) s.ghost.style.transform = `translate(${point.x + 10}px, ${point.y + 10}px)`;

  if (event.timeStamp - lastHitTestAt < HITTEST_THROTTLE_MS) return;
  lastHitTestAt = event.timeStamp;

  const located = locateTile(point.x, point.y);
  const tiles: DragTile[] = located
    ? [{ id: located.id, kind: located.kind, rect: located.el.getBoundingClientRect() }]
    : [];
  const intent = hitTest(point, s.sourceId, tiles);

  if (intent.kind !== "merge") {
    clearTarget(s);
    return;
  }
  if (s.targetId === intent.targetId) {
    // Same target as last tick: staying within slop leaves the running timer alone (§11: "moving
    // off the target or beyond the slop cancels and restarts the timer").
    if (s.dwellAnchor && !stillWithinSlop(s.dwellAnchor, point)) restartDwell(s, point);
    return;
  }
  s.targetId = intent.targetId;
  s.targetKind = intent.targetKind;
  s.targetEl = located?.el ?? null;
  restartDwell(s, point);
}

function restartDwell(s: Session, point: Point): void {
  if (s.armTimer) clearTimeout(s.armTimer);
  s.armed = false;
  s.dwellAnchor = point;
  setDragView({ sourceId: s.sourceId, targetId: s.targetId, armed: false });
  s.armTimer = setTimeout(() => arm(s), DWELL_MS);
}

function clearTarget(s: Session): void {
  if (s.targetId === null) return;
  if (s.armTimer) clearTimeout(s.armTimer);
  s.armTimer = null;
  s.armed = false;
  s.targetId = null;
  s.targetKind = null;
  s.targetEl = null;
  s.dwellAnchor = null;
  setDragView({ sourceId: s.sourceId, targetId: null, armed: false });
}

/**
 * Fires exactly `DWELL_MS` after the timer last (re)started. §11: reduced motion keeps this timer
 * unchanged — only the visual ring is affected (it is a plain class toggle keyed off `armed`, so
 * there is nothing to lag: it cannot render before this callback runs).
 */
function arm(s: Session): void {
  if (session !== s || !s.targetId) return;
  // Re-validate against the live DOM rather than a cached rect or a stale store read: a project
  // can crash and vanish during the dwell itself, with no pointermove in between to catch it.
  if (!document.body.contains(s.targetEl) || !findProject(s.sourceId)) {
    cancelSession();
    return;
  }
  s.armed = true;
  setDragView({ sourceId: s.sourceId, targetId: s.targetId, armed: true });
}

/**
 * §11 Motion: the ghost is one detached node, written imperatively outside React — compact
 * (~180px: name plus a status dot), positioned at cursor + (10, 10). `pointer-events-none` keeps
 * it out of `elementFromPoint`'s way. Reuses `STATUS_TONE` (§11 tokens, never raw hex) so the dot
 * matches the card it was lifted from.
 */
function createGhost(sourceId: string, point: Point): HTMLDivElement | null {
  const project = findProject(sourceId);
  if (!project) return null;
  const el = document.createElement("div");
  el.className =
    "pointer-events-none fixed left-0 top-0 z-50 flex w-[180px] items-center gap-2 " +
    "rounded-md border border-white/10 bg-surface px-2.5 py-1.5 text-sm text-text shadow-lg";
  el.style.transform = `translate(${point.x + 10}px, ${point.y + 10}px)`;
  const dot = document.createElement("span");
  dot.className = `size-1.5 shrink-0 rounded-full bg-current ${STATUS_TONE[project.status]}`;
  const label = document.createElement("span");
  label.className = "truncate";
  label.textContent = project.name;
  el.append(dot, label);
  document.body.appendChild(el);
  return el;
}

function onPointerUp(): void {
  const s = session;
  if (!s) return;
  const commit = s.armed && s.targetId !== null && s.targetKind !== null;
  const { sourceId, targetId, targetKind, targetEl, moved } = s;
  teardown();
  if (moved) suppressNextClick();
  if (commit && targetId && targetKind) void commitDrop(sourceId, targetId, targetKind, targetEl);
}

function onPointerCancel(): void {
  cancelSession();
}

function onWindowBlur(): void {
  cancelSession();
}

function onKeyDown(event: KeyboardEvent): void {
  if (event.key === "Escape") cancelSession();
}

/** Reset without committing — `pointercancel`, `blur`, `Escape`, a vanished source/target, or a
 *  `pointerup` that never armed. */
function cancelSession(): void {
  const moved = session?.moved ?? false;
  teardown();
  if (moved) suppressNextClick();
}

function teardown(): void {
  const s = session;
  if (!s) return;
  if (s.armTimer) clearTimeout(s.armTimer);
  s.ghost?.remove();
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerCancel);
  window.removeEventListener("keydown", onKeyDown);
  window.removeEventListener("blur", onWindowBlur);
  session = null;
  setDragView({ sourceId: null, targetId: null, armed: false });
}

/** Suppresses the trailing synthetic `click` a drag's `pointerup` produces — otherwise dropping a
 *  card also fires whatever it landed on (a folder's open/close button, Run, ...). */
function suppressNextClick(): void {
  window.addEventListener(
    "click",
    (event) => {
      event.stopPropagation();
      event.preventDefault();
    },
    { capture: true, once: true },
  );
}
