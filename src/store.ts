/**
 * Global store for project status and logs (SPEC.md §13).
 *
 * It holds the registry snapshot from the single `get_projects()` call, the corrupt-registry
 * banner, and the per-project log lines.
 *
 * SPEC.md §7 is explicit: the two event listeners — `status-changed` and `log-lines` — are
 * registered **once at app startup**, here, and never inside `LogPanel`. A listener that mounts
 * with the panel would lose every line emitted while the panel was closed.
 */
import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  clearLogBuffer,
  getLogBuffer,
  getProjects,
  getRegistryError,
  runProject,
  stopProject,
} from "./api";
import type {
  LogLine,
  LogLinesPayload,
  ProjectView,
  RegistryError,
  StatusChangedPayload,
} from "./types";

/** Mirrors the Rust ring buffer (SPEC.md §8) so the panel's copy can never outgrow it. */
export const LOG_BUFFER_LIMIT = 500;

export interface HangarState {
  projects: ProjectView[];
  registryError: RegistryError | null;
  loading: boolean;
  loadError: string | null;
  /** Per-project log lines, fed by the global `log-lines` listener and the backfill on open. */
  logs: Record<string, LogLine[]>;
  /** Which project's slide-over is open (§11), or `null`. */
  openLogsFor: string | null;
  /** Last command error — §7: errors surface as toasts. */
  toast: string | null;
}

let state: HangarState = {
  projects: [],
  registryError: null,
  loading: true,
  loadError: null,
  logs: {},
  openLogsFor: null,
  toast: null,
};

const listeners = new Set<() => void>();

function setState(patch: Partial<HangarState>): void {
  state = { ...state, ...patch };
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): HangarState {
  return state;
}

export function useHangarStore(): HangarState {
  return useSyncExternalStore(subscribe, getSnapshot);
}

function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  return String(err);
}

export function setToast(message: string | null): void {
  setState({ toast: message });
}

/** One initial fetch at startup. All later status changes arrive via events (§7) — never polling. */
export async function loadRegistry(): Promise<void> {
  setState({ loading: true, loadError: null });
  try {
    const [projects, registryError] = await Promise.all([
      getProjects(),
      getRegistryError(),
    ]);
    setState({ projects, registryError, loading: false });
  } catch (err) {
    setState({ loading: false, loadError: errorMessage(err) });
  }
}

// ---------------------------------------------------------------------------------------------
// The two §7 event listeners — registered once, for the lifetime of the app
// ---------------------------------------------------------------------------------------------

function applyStatusChanged(payload: StatusChangedPayload): void {
  const previous = state.projects.find((p) => p.id === payload.projectId)?.status;
  const projects = state.projects.map((p) =>
    p.id === payload.projectId ? { ...p, status: payload.status } : p,
  );

  // SPEC.md §8 buffer lifecycle: cleared at the start of each Run, retained after exit/crash/stop.
  // Rust clears its ring buffer as the run begins, so the store must drop the previous run's lines
  // at the same moment or an open panel would show two runs stitched together. Keyed on the
  // transition out of stopped/crashed rather than on `starting`, so the earlier phases plans 004
  // and 006 add (updating / installing) keep their output.
  const runIsStarting =
    (previous === "stopped" || previous === "crashed") &&
    payload.status !== "stopped" &&
    payload.status !== "crashed";

  setState({
    projects,
    logs: runIsStarting ? { ...state.logs, [payload.projectId]: [] } : state.logs,
  });
}

/** Keeps only the newest `LOG_BUFFER_LIMIT` lines, exactly like the Rust ring buffer. */
function capBuffer(lines: LogLine[]): LogLine[] {
  return lines.length <= LOG_BUFFER_LIMIT ? lines : lines.slice(lines.length - LOG_BUFFER_LIMIT);
}

function appendLogLines(projectId: string, incoming: LogLine[]): void {
  if (incoming.length === 0) return;
  const existing = state.logs[projectId] ?? [];
  setState({
    logs: { ...state.logs, [projectId]: capBuffer([...existing, ...incoming]) },
  });
}

let listenersStarted = false;

/**
 * Registers both §7 listeners. Idempotent, and never unsubscribed: they must outlive every
 * component, so there is nothing to tear down before the app itself goes away.
 */
export function startEventListeners(): void {
  if (listenersStarted) return;
  listenersStarted = true;

  void listen<StatusChangedPayload>("status-changed", (event) => {
    applyStatusChanged(event.payload);
  });
  void listen<LogLinesPayload>("log-lines", (event) => {
    appendLogLines(event.payload.projectId, event.payload.lines);
  });
}

// ---------------------------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------------------------

/** §7 `run_project`: fire-and-forget; the error is the toast for a rejected Run. */
export async function startProject(projectId: string): Promise<void> {
  try {
    await runProject(projectId);
  } catch (err) {
    setToast(errorMessage(err));
  }
}

/**
 * §7 `stop_project`. The card flips to `stopping` from the backend's event, not from here — the
 * status is never guessed in the frontend (§7: all status UI is derived from `status-changed`).
 *
 * The promise settles only when the kill has been verified, so a rejection means `stop-failed`:
 * processes survived or the port is still answering. Both facts are in the toast.
 */
export async function stopProjectAction(projectId: string): Promise<void> {
  try {
    await stopProject(projectId);
  } catch (err) {
    setToast(errorMessage(err));
  }
}

/**
 * Merge a fetched backfill with the lines already received live (SPEC.md §8: "subscribe first,
 * then fetch, then merge — drop fetched lines already received live").
 *
 * Both arrays are windows onto the same ordered stream, so the join is the longest overlap
 * between the tail of `fetched` and the head of `live`; everything after it is new.
 */
export function mergeLogBuffers(fetched: LogLine[], live: LogLine[]): LogLine[] {
  const max = Math.min(fetched.length, live.length);
  for (let k = max; k > 0; k -= 1) {
    let matches = true;
    for (let i = 0; i < k; i += 1) {
      const a = fetched[fetched.length - k + i];
      const b = live[i];
      if (a.stream !== b.stream || a.line !== b.line) {
        matches = false;
        break;
      }
    }
    if (matches) return capBuffer([...fetched, ...live.slice(k)]);
  }
  // No overlap at all (e.g. the buffer was cleared between the two): keep both, newest last.
  return capBuffer([...fetched, ...live]);
}

/**
 * Opens the §11 slide-over and backfills from the Rust-owned buffer, which is the source of
 * truth (§8). Everything the store already held is replaced by it; only the lines that arrived
 * live *while the fetch was in flight* are kept and de-duplicated against its tail.
 */
export async function openLogs(projectId: string): Promise<void> {
  setState({ openLogsFor: projectId });
  const receivedBeforeFetch = (state.logs[projectId] ?? []).length;
  try {
    const fetched = await getLogBuffer(projectId);
    const arrivedDuringFetch = (state.logs[projectId] ?? []).slice(receivedBeforeFetch);
    setState({
      logs: {
        ...state.logs,
        [projectId]: mergeLogBuffers(fetched, arrivedDuringFetch),
      },
    });
  } catch (err) {
    setToast(errorMessage(err));
  }
}

export function closeLogs(): void {
  setState({ openLogsFor: null });
}

/** §8: the Clear button calls `clear_log_buffer` and clears the store. */
export async function clearLogs(projectId: string): Promise<void> {
  try {
    await clearLogBuffer(projectId);
    setState({ logs: { ...state.logs, [projectId]: [] } });
  } catch (err) {
    setToast(errorMessage(err));
  }
}
