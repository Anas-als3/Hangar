/**
 * Single source of truth for the wire shapes, mirroring the Rust structs in
 * `src-tauri/src/registry.rs` (which derive `#[serde(rename_all = "camelCase")]`).
 * SPEC.md §5 (data model) and §7 (frozen command/event contract).
 */

export type Status =
  | "stopped"
  | "updating"
  | "installing"
  | "starting"
  | "running"
  | "stopping"
  | "crashed"
  | "stop-failed";

export interface Project {
  /** nanoid */
  id: string;
  name: string;
  /** absolute folder path */
  path: string;
  /** "npm run dev" — free text, run through the platform shell */
  command: string;
  /** pinned per project; used for ready-check + browser URL */
  port: number;
  /** optional override; default `http://localhost:${port}` */
  url?: string;
  updateOnRun: boolean;
  readyTimeoutSec: number;
  /** internal */
  lastLockfileHash?: string;
  /** ISO — set when entering `starting` */
  lastRunAt?: string;
  /** Free-text scratchpad, user-owned; never parsed or acted on (SPEC.md §5). */
  notes?: string;
  /**
   * Detected from `package.json` dependencies — app-owned, never hand-edited. Refreshed on Add,
   * on Edit, and during the install phase (SPEC.md §5, added 2026-08-09).
   */
  stack?: ProjectStack;
  /** Opaque, generated; the folder IS the set of projects sharing it (SPEC.md §5, added 2026-08-10). */
  folderId?: string;
  /** The folder's display name, denormalised onto every member (SPEC.md §5, added 2026-08-10). */
  folderName?: string;
  /**
   * When `false`, reaching ready does not hand off to the browser — the status transition is
   * unaffected, only the tab is skipped. Absent/`true` is the default (SPEC.md §5 / §9 step 6,
   * added 2026-08-10).
   */
  openBrowserOnReady?: boolean;
}

/** SPEC.md §5 `Project.stack` / §7 `read_package_json`'s `stack` field. */
export interface ProjectStack {
  framework?: string;
  libraries: string[];
  /** ISO — when detection last ran. */
  detectedAt: string;
}

/** What the frontend receives; derived fields are computed by the backend, never persisted. */
export interface ProjectView extends Project {
  status: Status;
  pathExists: boolean;
}

/** §7: `NewProject = Project minus id/lastLockfileHash/lastRunAt`. Used by `add_project` (plan 005). */
export type NewProject = Omit<Project, "id" | "lastLockfileHash" | "lastRunAt">;

/** §7 `read_package_json` return shape — the Add/Edit dialog's script and port suggestions. */
export interface PackageJsonInfo {
  scripts: Record<string, string>;
  packageManager: "npm" | "pnpm" | "yarn";
  portSuggestion?: number;
  /** Always present, possibly empty (added 2026-08-09) — see `ProjectStack`. */
  stack: ProjectStack;
}

export interface LogLine {
  stream: "stdout" | "stderr" | "system";
  line: string;
}

/**
 * §7 `get_port_status` — SPEC.md §11 Ports panel snapshot (added 2026-08-10). One entry per
 * registered project; a lookup that fails or times out is never an error — it comes back as
 * `busy: true, listenerCount: 0, holder: undefined` (the "owner unknown" row).
 */
export interface PortStatus {
  projectId: string;
  port: number;
  busy: boolean;
  /** > 1 → Hangar names nobody and offers nothing. */
  listenerCount: number;
  /** Only present when `listenerCount === 1` and the lookup parsed. */
  holder?: PortHolder;
  /** ISO — shared by every row from one `get_port_status` call. */
  checkedAt: string;
}

/** §7 `PortHolder`. `command`/`startedAt`/`parentExited` are Unix only; undefined on Windows. */
export interface PortHolder {
  name: string;
  pid: number;
  command?: string;
  startedAt?: string;
  parentExited?: boolean;
  /** `false` → the (plan 042) free-port action must never be offered. */
  sameUser?: boolean;
}

/**
 * SPEC.md §11 "Doctor" (added 2026-08-11, plan 057) — `get_preflight`'s wire shape. Mirrors
 * `preflight::PreflightReport`/`PreflightFinding`/`Severity`.
 *
 * **A finding carries key NAMES only.** There is deliberately no field here — and none in the
 * Rust struct — capable of holding a `.env` value: a field that exists can be filled by a later
 * refactor, a field that does not exist cannot. Do not add one.
 */
export type PreflightSeverity = "blocker" | "warning" | "note";

export interface PreflightFinding {
  /** Stable across calls for the same fact. */
  id: string;
  severity: PreflightSeverity;
  /** One human line, built from key names, filenames and version strings only. */
  message: string;
  /** Relative to the project folder, or the project's own path for a folder-level finding. */
  file: string;
}

export interface PreflightReport {
  projectId: string;
  /** Empty is the common, quiet case. In check order — never sorted by severity (§11). */
  findings: PreflightFinding[];
  /** ISO — shared by every report from one `get_preflight` call. */
  checkedAt: string;
}

/**
 * SPEC.md §11 "Launch line" (added 2026-08-11, plan 060) — `get_vcs_status`'s wire shape. Mirrors
 * `vcs::VcsStatus`/`vcs::VcsState`.
 *
 * **There is deliberately no `behind` field**, here or in the Rust struct. Hangar never fetches, so
 * any "behind" number would be as old as the user's last manual fetch and would read as current — a
 * stale "you are up to date" is worse than silence. A field that does not exist cannot be filled.
 */
export type VcsState =
  /** Looked, and there is genuinely nothing to say. Silent. */
  | "not-a-repo"
  /** `git status` answered — `ahead`/`uncommitted` carry the answer. */
  | "checked"
  /**
   * `git status` did **not** answer (git missing, timed out, non-zero exit). **Not "clean".** The
   * line must render something for this, or a check that could not run renders as a clean bill of
   * health — the exact bug SPEC.md §11 forbids for the Doctor panel's dependency check.
   */
  | "unavailable";

export interface VcsStatus {
  projectId: string;
  state: VcsState;
  /**
   * Unpushed commits: on `HEAD`, absent from the **local** remote-tracking ref. Exact — both refs
   * are local facts. Absent when `state !== "checked"`, and when the branch has no upstream, is
   * detached, or the repo has no commits: nothing to count is not the same as counting zero.
   */
  ahead?: number;
  /** How many paths a porcelain status listed. A count — never a name, never a diff. */
  uncommitted?: number;
  /** Why the check could not run. Only ever present for `"unavailable"`. */
  detail?: string;
  /** ISO — shared by every row from one `get_vcs_status` call. */
  checkedAt: string;
}

/** §7 event payload — emitted on every transition. */
export interface StatusChangedPayload {
  projectId: string;
  status: Status;
  message?: string;
}

/** §7 event payload — batched, at most one flush every 100 ms. */
export interface LogLinesPayload {
  projectId: string;
  lines: LogLine[];
}

/** §7 `get_settings` / `set_settings`. */
export interface Settings {
  editorCommand: string;
  /**
   * SPEC.md §11 / plan 059 — the osv.dev dependency check in the Doctor panel. **Off by default**;
   * mirrors `registry::Settings::check_dependencies`. When on, opening Doctor sends the package
   * names and versions from each project's `package-lock.json` to osv.dev, and nothing else.
   */
  checkDependencies: boolean;
}

/**
 * Corrupt-registry report behind the §11 persistent banner (SPEC.md §4, §12).
 * Not part of the frozen §7 list — see the note on `get_registry_error` in `api.ts`.
 */
export interface RegistryError {
  backupPath: string | null;
  error: string;
}

/**
 * SPEC.md §18 / plan 053 — `get_github_status`/`set_github_token`'s wire shape. Mirrors
 * `commands::GithubStatus`/`GithubConnectionState`. `KeychainDenied` is distinct from
 * `Disconnected` on purpose: "a denied keychain must never render as 'no token'" (§18).
 */
export type GithubConnectionState =
  | "disconnected"
  | "keychain-denied"
  | "connected"
  | "invalid"
  | "insufficient-scope"
  | "rate-limited"
  | "secondary-rate-limited"
  | "offline";

export interface GithubStatus {
  state: GithubConnectionState;
  username?: string;
  scopes?: string[];
  /** Human-readable, secret-free explanation for every non-`connected` state. */
  detail?: string;
  /** ISO — present only for `rate-limited`. */
  resetAt?: string;
  /** Present only for `secondary-rate-limited`. */
  retryAfterSec?: number;
  /** Lets the panel say "Reconnect" instead of "Connect" once a token existed and stopped working. */
  hadStoredToken?: boolean;
}
