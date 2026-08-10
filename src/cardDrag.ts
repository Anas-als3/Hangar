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
