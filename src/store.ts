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
  addProject,
  clearLogBuffer,
  getLogBuffer,
  getProjects,
  getRegistryError,
  openInBrowser,
  openInEditor,
  removeProject,
  runProject,
  setSettings,
  stopProject,
  updateProject,
} from "./api";
import type {
  LogLine,
  LogLinesPayload,
  NewProject,
  Project,
  ProjectView,
  RegistryError,
  Settings,
  StatusChangedPayload,
} from "./types";

/**
 * Which dialog (SPEC.md §10/§11) is open, or `null`. `edit` carries the full `Project` being
 * edited so `AddEditDialog` can pre-fill without a second fetch.
 */
export type DialogState =
  | { kind: "add" }
  | { kind: "edit"; project: Project }
  | { kind: "settings" }
  | null;

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
  /** Which dialog (add/edit/settings) is open — see `DialogState`. */
  dialog: DialogState;
}

let state: HangarState = {
  projects: [],
  registryError: null,
  loading: true,
  loadError: null,
  logs: {},
  openLogsFor: null,
  toast: null,
  dialog: null,
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

/**
 * Looks up a project outside a React render — e.g. from a `ProjectCard` menu action, which the
 * frozen §7 `MENU_ITEMS` shape (SPEC.md §13) hands only an id, not the full `ProjectView`.
 */
export function findProject(id: string): ProjectView | undefined {
  return state.projects.find((p) => p.id === id);
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

  // §7: "message carries e.g. the crash reason". A Run is fire-and-forget, so everything that goes
  // wrong *after* it returns — the §9 step 5 wrong-script diagnosis, the step 7 ready-timeout —
  // can only reach the user through this event. Scoped to `crashed`: `stop-failed` already toasts
  // from the rejected `stop_project` call, and toasting both would double up.
  if (payload.status === "crashed" && payload.message) {
    setToast(payload.message);
  }
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
 * §7 `open_in_browser` — the overflow-menu action. The tab that opens by itself when a project
 * turns `running` (§9 step 6) does not come through here; Rust opens it directly.
 */
export async function openInBrowserAction(projectId: string): Promise<void> {
  try {
    await openInBrowser(projectId);
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

// ---------------------------------------------------------------------------------------------
// Dialogs (§10/§11) — AddEditDialog and SettingsDialog are pure views of `dialog`.
// ---------------------------------------------------------------------------------------------

export function openAddDialog(): void {
  setState({ dialog: { kind: "add" } });
}

export function openEditDialog(project: Project): void {
  setState({ dialog: { kind: "edit", project } });
}

export function openSettingsDialog(): void {
  setState({ dialog: { kind: "settings" } });
}

export function closeDialog(): void {
  setState({ dialog: null });
}

/** §7 `add_project`. On success, refresh the registry and close the dialog; on rejection, toast. */
export async function addProjectAction(input: NewProject): Promise<boolean> {
  try {
    await addProject(input);
    await loadRegistry();
    setState({ dialog: null });
    return true;
  } catch (err) {
    setToast(errorMessage(err));
    return false;
  }
}

/**
 * §7 `update_project`. Rejected if the project is not `stopped`/`crashed` — callers must run
 * `stopIfRunningWithConfirm` first (see below).
 */
export async function updateProjectAction(project: Project): Promise<boolean> {
  try {
    await updateProject(project);
    await loadRegistry();
    setState({ dialog: null });
    return true;
  } catch (err) {
    setToast(errorMessage(err));
    return false;
  }
}

/** §7 `remove_project`. Same not-running precondition as update. */
export async function removeProjectAction(projectId: string): Promise<void> {
  try {
    await removeProject(projectId);
    await loadRegistry();
  } catch (err) {
    setToast(errorMessage(err));
  }
}

/** §10 step 7 — overflow-menu "Open in editor". A rejection is the "couldn't run" toast. */
export async function openInEditorAction(projectId: string): Promise<void> {
  try {
    await openInEditor(projectId);
  } catch (err) {
    setToast(errorMessage(err));
  }
}

/** §7 `set_settings`. Closes the Settings dialog on success. */
export async function saveSettingsAction(settings: Settings): Promise<boolean> {
  try {
    await setSettings(settings);
    setState({ dialog: null });
    return true;
  } catch (err) {
    setToast(errorMessage(err));
    return false;
  }
}

/**
 * §6/§10 step 7: Remove/Edit on a project that is not `stopped`/`crashed` must confirm, then stop
 * and wait for verified death, before the caller applies the update/remove. Returns whether it is
 * now safe to proceed. Uses `stopProjectAction` (not the raw `stopProject`) so the store's own
 * status handling stays the single path; success is read back from the store, which
 * `status-changed` has by then updated to `stopped` or `stop-failed`.
 */
export async function stopIfRunningWithConfirm(project: ProjectView): Promise<boolean> {
  if (project.status === "stopped" || project.status === "crashed") return true;
  if (!window.confirm(`${project.name} is running. Stop it first?`)) return false;
  await stopProjectAction(project.id);
  const after = state.projects.find((p) => p.id === project.id)?.status;
  return after === "stopped" || after === "crashed";
}
