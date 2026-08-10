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
import { lastRunLabel } from "./status";
import type {
  LogLine,
  LogLinesPayload,
  NewProject,
  Project,
  ProjectView,
  RegistryError,
  Settings,
  Status,
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
  | { kind: "move-folder"; project: Project }
  | null;

/** §11: the neutral toast tone for non-error confirmations (the move-to-folder toast). Defaults
 *  to `"error"` everywhere `setToast` is already called, so those call sites stay unchanged. */
export type ToastTone = "error" | "neutral";

/** Mirrors the Rust ring buffer (SPEC.md §8) so the panel's copy can never outgrow it. */
export const LOG_BUFFER_LIMIT = 500;

/**
 * Plan 030 card-drag view state — deliberately only these three fields. Pointer coordinates never
 * enter the store: `cardDrag.ts` tracks them in a module-level session, and only pushes here on
 * the handful of transitions per drag (start, target change, arm), never on every pointermove.
 */
export interface DragViewState {
  sourceId: string | null;
  targetId: string | null;
  armed: boolean;
}

export interface HangarState {
  projects: ProjectView[];
  registryError: RegistryError | null;
  loading: boolean;
  loadError: string | null;
  /** Per-project log lines, fed by the global `log-lines` listener and the backfill on open. */
  logs: Record<string, LogLine[]>;
  /** Phases actually observed per project this run (plan 027) — ephemeral view state that
   *  outlives a card unmount, so search-filtering a card out and back cannot blank the §11
   *  phase strip. Never persisted: this is not a `Project` field. */
  phasesSeen: Record<string, string[]>;
  /** Which project's slide-over is open (§11), or `null`. */
  openLogsFor: string | null;
  /** Which project's notes slide-over is open (§11), or `null`. */
  notesFor: string | null;
  /** Last command error — §7: errors surface as toasts. */
  toast: string | null;
  /** §11: the current toast's styling. Defaults to `"error"` — see `ToastTone`. */
  toastTone: ToastTone;
  /** Which dialog (add/edit/settings) is open — see `DialogState`. */
  dialog: DialogState;
  /** Ephemeral header search term (plan 017) — never persisted, filters by name only. */
  search: string;
  /** §11 "Opening a folder": which folders are expanded. Ephemeral view state — never written to
   *  `projects.json`, and `loadRegistry()` must never touch this field (see below), or the folder
   *  you just opened would collapse every time a window-focus reload fires. */
  openFolders: Set<string>;
  /** Plan 030 card-drag view state — see `DragViewState`. */
  drag: DragViewState;
}

let state: HangarState = {
  projects: [],
  registryError: null,
  loading: true,
  loadError: null,
  logs: {},
  phasesSeen: {},
  openLogsFor: null,
  notesFor: null,
  toast: null,
  toastTone: "error",
  dialog: null,
  search: "",
  openFolders: new Set(),
  drag: { sourceId: null, targetId: null, armed: false },
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

/**
 * `tone` defaults to `"error"` — the styling every existing call site already gets, unchanged.
 * The move-to-folder confirmation is the one caller that passes `"neutral"` explicitly.
 */
export function setToast(message: string | null, tone: ToastTone = "error"): void {
  setState({ toast: message, toastTone: tone });
}

/**
 * Plan 030's only store surface for the card-drag gesture — `cardDrag.ts` calls this a handful of
 * times per drag (start, target change, arm, teardown), never per pointermove. `ProjectCard` and
 * `FolderTile` read `drag` back via `useHangarStore` to dim the source and ring an armed target.
 */
export function setDragView(next: DragViewState): void {
  setState({ drag: next });
}

/** Header search box (plan 017) — ephemeral view state, never written to disk. */
export function setSearch(value: string): void {
  setState({ search: value });
}

/**
 * §11 "Opening a folder": toggles one folder's open/closed state. Ephemeral view state — never
 * persisted, never reset by `loadRegistry` (that function's own `setState` calls below never
 * mention `openFolders`, so a merge patch leaves it untouched).
 *
 * The `stop-failed` auto-expand-and-cannot-collapse rule is deliberately NOT enforced here: it is
 * a derived predicate at render time (`FolderTile`/`ProjectGrid` OR this bit with "has a
 * stop-failed member"), so toggling never gets a folder stuck — the underlying bit still flips,
 * it is only the rendered `isOpen` that the predicate overrides.
 */
export function toggleFolder(folderId: string): void {
  const next = new Set(state.openFolders);
  if (next.has(folderId)) next.delete(folderId);
  else next.add(folderId);
  setState({ openFolders: next });
}

/**
 * Unconditional close — used by the open band's own Esc handler (§11), never a raw
 * `toggleFolder`. A band can be visible while its id is absent from `openFolders` (the
 * stop-failed auto-expand override), and a blind toggle would then *add* the id — recording the
 * opposite of what Esc means. Closing is idempotent: a no-op if the id isn't present.
 */
export function closeFolder(folderId: string): void {
  if (!state.openFolders.has(folderId)) return;
  const next = new Set(state.openFolders);
  next.delete(folderId);
  setState({ openFolders: next });
}

/** Case-insensitive substring match on name, order preserved (SPEC.md §11). */
export function filterProjects(projects: ProjectView[], search: string): ProjectView[] {
  const q = search.trim().toLowerCase();
  if (q === "") return projects;
  return projects.filter((p) => p.name.toLowerCase().includes(q));
}

/**
 * What the grid actually renders under an active search: a project stays visible if it matches
 * OR it is currently non-idle (anything but `stopped`/`crashed`) — a running project must never
 * be unmounted by a search that doesn't name it, which would reset its `PhaseStrip` and uptime.
 * A single `filter` over the original array, never "matches" concatenated with "running": SPEC.md
 * §11 forbids automatic re-sorting, and concatenation would reorder.
 */
export function visibleProjects(projects: ProjectView[], search: string): ProjectView[] {
  const q = search.trim().toLowerCase();
  if (q === "") return projects;
  return projects.filter(
    (p) => p.name.toLowerCase().includes(q) || (p.status !== "stopped" && p.status !== "crashed"),
  );
}

/** Header count (SPEC.md §11): projects currently `running`. Renders nothing when zero. */
export function runningCount(projects: ProjectView[]): number {
  return projects.filter((p) => p.status === "running").length;
}

// ---------------------------------------------------------------------------------------------
// Folders (SPEC.md §11 "Folders" / "Opening a folder", plan 029) — derivation only. The array in
// `projects.json` is never rewritten by any of this; a folder's position is *derived* from where
// its earliest member sits in `projects`.
// ---------------------------------------------------------------------------------------------

export interface ProjectGridItem {
  kind: "project";
  project: ProjectView;
}

export interface FolderGridItem {
  kind: "folder";
  id: string;
  name: string;
  /** Array order — the same order the dot row and the open band render members in. */
  members: ProjectView[];
}

export type GridItem = ProjectGridItem | FolderGridItem;

/**
 * The one walk (§11): a project carrying a `folderId` is not drawn as its own tile; the first
 * time the walk reaches any member of a folder, that folder's tile is emitted in that position,
 * and every later member is folded into it instead of emitted again. No `sort`, no `concat` of
 * two filtered lists — the output order is exactly `projects`' order with folder members merged
 * into their folder's first position.
 *
 * Under an active search, §11 says folders dissolve: every project `visibleProjects` would show
 * renders flat, as a plain card, in array order.
 */
export function gridItems(projects: ProjectView[], search: string): GridItem[] {
  if (search.trim() !== "") {
    return visibleProjects(projects, search).map(
      (project): GridItem => ({ kind: "project", project }),
    );
  }
  const items: GridItem[] = [];
  const folders = new Map<string, FolderGridItem>();
  for (const project of projects) {
    if (!project.folderId) {
      items.push({ kind: "project", project });
      continue;
    }
    const folder = folders.get(project.folderId);
    if (folder) {
      folder.members.push(project);
      continue;
    }
    // First sighting of this folderId is, by construction, its earliest member — §5's tiebreak
    // for the displayed name when members ever disagree.
    const created: FolderGridItem = {
      kind: "folder",
      id: project.folderId,
      name: project.folderName ?? "",
      members: [project],
    };
    folders.set(project.folderId, created);
    items.push(created);
  }
  return items;
}

/** §11: the four transitional/active statuses folded into the folder tile's "in progress" bucket. */
const IN_PROGRESS_STATUSES: ReadonlySet<Status> = new Set<Status>([
  "updating",
  "installing",
  "starting",
  "stopping",
]);

/**
 * §11 folder aggregate line: counts, never a status. Fixed severity order — `n stop-failed`,
 * `n crashed`, `n running`, `n in progress` — joined so truncation can only drop the harmless
 * end, zero-count fragments omitted. When every member is `stopped`, shows the most recently run
 * member's last-run relative time instead, matching the card's own time-slot rule.
 */
export function folderSummary(members: ProjectView[]): string {
  let stopFailed = 0;
  let crashed = 0;
  let running = 0;
  let inProgress = 0;
  let allStopped = true;
  for (const member of members) {
    if (member.status !== "stopped") allStopped = false;
    if (member.status === "stop-failed") stopFailed += 1;
    else if (member.status === "crashed") crashed += 1;
    else if (member.status === "running") running += 1;
    else if (IN_PROGRESS_STATUSES.has(member.status)) inProgress += 1;
  }
  if (allStopped) {
    const mostRecent = members.reduce<ProjectView | null>((latest, m) => {
      if (!m.lastRunAt) return latest;
      if (!latest?.lastRunAt) return m;
      return Date.parse(m.lastRunAt) > Date.parse(latest.lastRunAt) ? m : latest;
    }, null);
    return lastRunLabel(mostRecent?.lastRunAt);
  }
  const fragments: string[] = [];
  if (stopFailed > 0) fragments.push(`${stopFailed} stop-failed`);
  if (crashed > 0) fragments.push(`${crashed} crashed`);
  if (running > 0) fragments.push(`${running} running`);
  if (inProgress > 0) fragments.push(`${inProgress} in progress`);
  return fragments.join(" · ");
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

/** Mirrors `PhaseKey` in `PhaseStrip.tsx`, kept local: the dependency runs store -> component. */
function isPhaseStatus(status: Status): boolean {
  return (
    status === "updating" ||
    status === "installing" ||
    status === "starting" ||
    status === "running"
  );
}

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

  // Plan 027: the phases actually observed this run, keyed on the same `runIsStarting`
  // transition as the log buffer above, so the two can never drift apart.
  const basePhases = runIsStarting ? [] : (state.phasesSeen[payload.projectId] ?? []);
  const nextPhases =
    isPhaseStatus(payload.status) && !basePhases.includes(payload.status)
      ? [...basePhases, payload.status]
      : basePhases;

  setState({
    projects,
    logs: runIsStarting ? { ...state.logs, [payload.projectId]: [] } : state.logs,
    phasesSeen: { ...state.phasesSeen, [payload.projectId]: nextPhases },
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
    // SPEC.md §5: pathExists must refresh "when Run is clicked" — a rejection is often exactly
    // that check failing, so the card should pick up the warning state that caused it.
    await loadRegistry();
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

// ---------------------------------------------------------------------------------------------
// Notes (§11) — a slide-over like Logs, opened from the overflow menu. Autosaved; no dialog.
// ---------------------------------------------------------------------------------------------

export function openNotes(projectId: string): void {
  setState({ notesFor: projectId });
}

export function closeNotes(): void {
  setState({ notesFor: null });
}

/**
 * §7 `update_project` carries the whole record — there is no dedicated notes command (§7 is
 * frozen). The backend now exempts notes-only changes from the running-project guard (plan 020
 * revision), but only if the payload actually IS notes-only: takes a project id rather than a
 * `Project`, and reads the current record straight out of the store at save time, instead of
 * trusting a snapshot the caller may have captured when the notes panel first opened. A stale
 * snapshot of any other field (say, `port`, changed elsewhere while the panel sat open) would
 * make the backend's comparison see a non-notes change and reject the save while running.
 *
 * Refreshes the registry on success so the edited textarea's project prop stays in sync; toasts
 * on rejection, same shape as the other actions.
 */
export async function saveNotesAction(projectId: string, notes: string): Promise<void> {
  const project = findProject(projectId);
  if (!project) return;
  try {
    await updateProject({ ...project, notes: notes === "" ? undefined : notes });
    await loadRegistry();
  } catch (err) {
    setToast(errorMessage(err));
  }
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

/** §11 "Move to folder…" — the overflow-menu item `ProjectCard` adds (plan 029). */
export function openMoveToFolderDialog(project: Project): void {
  setState({ dialog: { kind: "move-folder", project } });
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

// ---------------------------------------------------------------------------------------------
// Folders (§11 "Folders", §5 folder semantics) — moveToFolder is the required non-drag route
// both into and out of a folder; rename/ungroup are the folder tile's own menu actions.
// ---------------------------------------------------------------------------------------------

/**
 * `target` is a discriminated union rather than a bare string: on the wire an existing folder id
 * and a freshly typed folder name are both strings, so collapsing them into one `string | null`
 * parameter would make "is this id-shaped or name-shaped" a runtime guess. `MoveToFolderDialog`
 * already has both the id and the display name of every folder it lists, so passing both costs it
 * nothing and saves a lookup here.
 */
export type FolderTarget =
  | { kind: "existing"; folderId: string; folderName: string }
  | { kind: "new"; name: string }
  | { kind: "none" };

/** §5: folder ids are opaque and generated, never derived from the name. `crypto.randomUUID()` is
 *  available in WKWebView (§4/scope: no id library added for this one call site). */
function generateFolderId(): string {
  return crypto.randomUUID();
}

/**
 * §11 "Move to folder…" — reads the project fresh via `findProject` at call time, never from a
 * value captured earlier, copying `saveNotesAction`'s defence exactly: a stale snapshot taken
 * before an `await` (e.g. from when the dialog first opened) could roll back whatever a run wrote
 * to some other field in the meantime.
 */
export async function moveToFolder(projectId: string, target: FolderTarget): Promise<boolean> {
  const project = findProject(projectId);
  if (!project) return false;
  const patch =
    target.kind === "none"
      ? { folderId: undefined, folderName: undefined }
      : target.kind === "new"
        ? { folderId: generateFolderId(), folderName: target.name }
        : { folderId: target.folderId, folderName: target.folderName };
  try {
    await updateProject({ ...project, ...patch });
    await loadRegistry();
    return true;
  } catch (err) {
    setToast(errorMessage(err));
    return false;
  }
}

/**
 * §11 folder tile menu — Rename. N `updateProject` calls, one per member (§5: `folderName` is
 * denormalised, there is no folder record to write once). If a call partway through fails, §5
 * already specifies the recovery: "the earliest member in array order supplies the displayed
 * name, with the next rename repairing the rest" — no special rollback needed here.
 */
export async function renameFolder(folderId: string, name: string): Promise<void> {
  const members = state.projects.filter((p) => p.folderId === folderId);
  try {
    for (const member of members) {
      await updateProject({ ...member, folderName: name });
    }
  } catch (err) {
    setToast(errorMessage(err));
  } finally {
    // Plan 033 defect 3: reload on BOTH paths. A mid-sequence rejection must never leave the
    // grid showing the pre-rename name while disk holds a partial write — no retry/rollback
    // needed (§5's next-rename-repairs-it recovery already covers that), just an accurate view.
    await loadRegistry();
  }
}

/** §11 folder tile menu — Ungroup. Clears both folder fields on every member; the folder record
 *  is only ever the set of projects sharing an id, so there is nothing else to clean up (§5). */
export async function ungroupFolder(folderId: string): Promise<void> {
  const members = state.projects.filter((p) => p.folderId === folderId);
  try {
    for (const member of members) {
      await updateProject({ ...member, folderId: undefined, folderName: undefined });
    }
  } catch (err) {
    setToast(errorMessage(err));
  } finally {
    // Plan 033 defect 3: reload on BOTH paths — see renameFolder's comment above. A partial
    // ungroup must not leave the tile showing the old member count while disk already lost some.
    await loadRegistry();
  }
}
