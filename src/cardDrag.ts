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
