/**
 * Shared §11 status vocabulary — lifted out of `ProjectCard.tsx` (plan 029) so the folder tile's
 * dot row can reuse the exact same palette and label text instead of forking it.
 *
 * No behaviour change, no value change from the definitions that used to live in ProjectCard.
 */
import type { Status } from "./types";

export const STATUS_LABEL: Record<Status, string> = {
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
export const STATUS_TONE: Record<Status, string> = {
  stopped: "text-status-stopped",
  updating: "text-status-active hangar-pulse",
  installing: "text-status-active hangar-pulse",
  starting: "text-status-active hangar-pulse",
  running: "text-status-running",
  stopping: "text-status-active hangar-pulse",
  crashed: "text-status-danger",
  "stop-failed": "text-status-danger",
};

/** Relative last-run time. Coarse on purpose — §11 forbids ticking seconds. */
export function lastRunLabel(iso: string | undefined): string {
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
