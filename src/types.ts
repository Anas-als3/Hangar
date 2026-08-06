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
}

export interface LogLine {
  stream: "stdout" | "stderr" | "system";
  line: string;
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
}

/**
 * Corrupt-registry report behind the §11 persistent banner (SPEC.md §4, §12).
 * Not part of the frozen §7 list — see the note on `get_registry_error` in `api.ts`.
 */
export interface RegistryError {
  backupPath: string | null;
  error: string;
}
