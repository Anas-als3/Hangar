/**
 * The only file that calls `invoke()` (SPEC.md §7, §13). Components import from here.
 *
 * M1 wired the read-only slice of the frozen API (plus `get_settings`/`set_settings`); M2 added
 * `run_project`, `get_log_buffer` and `clear_log_buffer`; M3 added `stop_project`; M4 added
 * `open_in_browser`. M5 (this plan) adds the rest of §7: `add_project`, `update_project`,
 * `remove_project`, `read_package_json`, `open_in_editor` — under exactly those names.
 */
import { invoke } from "@tauri-apps/api/core";
import type {
  BuildFreshness,
  GithubStatus,
  LogLine,
  NewProject,
  PackageJsonInfo,
  PortStatus,
  PreflightReport,
  Project,
  ProjectView,
  RegistryError,
  Settings,
  VcsStatus,
} from "./types";

export function getProjects(): Promise<ProjectView[]> {
  return invoke<ProjectView[]>("get_projects");
}

/** §7 `add_project` / §10 — the duplicate-port rejection (naming the owner) happens in Rust. */
export function addProject(input: NewProject): Promise<ProjectView> {
  return invoke<ProjectView>("add_project", { input });
}

/**
 * §7 `update_project`. Takes the FULL `Project` (id included), not a patch — callers must spread
 * the project being edited and override only the fields the dialog changed, so `lastLockfileHash`
 * / `lastRunAt` survive an edit unchanged. Rejected if the project is not `stopped`/`crashed`
 * (§6) — the caller must confirm-and-stop first (see `stopIfRunningWithConfirm` in `store.ts`).
 */
export function updateProject(project: Project): Promise<ProjectView> {
  return invoke<ProjectView>("update_project", { project });
}

/** §7 `remove_project` — rejected with a message if status ∉ {stopped, crashed} (§6). */
export function removeProject(id: string): Promise<void> {
  return invoke<void>("remove_project", { id });
}

/**
 * §7 `read_package_json` — the Add dialog's script list, package-manager detection and port
 * suggestion (§10 steps 2-4, 6). Never rejects: a missing/unparseable `package.json` comes back
 * as empty scripts, which is what lets the dialog fall back to manual command + port entry.
 */
export function readPackageJson(path: string): Promise<PackageJsonInfo> {
  return invoke<PackageJsonInfo>("read_package_json", { path });
}

/**
 * §7 `open_in_editor` / §10 step 7 — runs `<editorCommand> <path>` through the Rust spawn helper.
 * A rejection is the "Couldn't run '<editor>' " toast.
 */
export function openInEditor(id: string): Promise<void> {
  return invoke<void>("open_in_editor", { id });
}

/**
 * §7: fire-and-forget — all progress arrives via the `status-changed` and `log-lines` events.
 * A rejected Run (wrong status, missing folder, spawn failure) rejects this promise instead.
 */
export function runProject(id: string): Promise<void> {
  return invoke<void>("run_project", { id });
}

/**
 * §7 `stop_project`. Valid in every active phase, and from `stop-failed` as a retry.
 *
 * The promise resolves only once Rust has killed the tree AND verified it (process death first,
 * then the port — §8). A rejection is the `stop-failed` toast; the card's own red pill comes from
 * the `status-changed` event, not from here.
 */
export function stopProject(id: string): Promise<void> {
  return invoke<void>("stop_project", { id });
}

/**
 * §7 `open_in_browser` — the overflow-menu action only.
 *
 * The automatic tab on entering `running` (§9 step 6) is opened by Rust, not by this call: §4 puts
 * the opener plugin on the Rust side so it bypasses the ACL, and routing it through the webview
 * would need a capability entry the spec does not grant.
 */
export function openInBrowser(id: string): Promise<void> {
  return invoke<void>("open_in_browser", { id });
}

/** §8: Rust owns the buffer; the panel backfills from it on open. */
export function getLogBuffer(id: string): Promise<LogLine[]> {
  return invoke<LogLine[]>("get_log_buffer", { id });
}

export function clearLogBuffer(id: string): Promise<void> {
  return invoke<void>("clear_log_buffer", { id });
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export function setSettings(s: Settings): Promise<void> {
  return invoke<void>("set_settings", { s });
}

/**
 * Addition to §7 (documented deviation): §4/§12 require a persistent banner naming the
 * `.broken-<timestamp>` backup and the parse error, and the frozen list has no vehicle for it.
 */
export function getRegistryError(): Promise<RegistryError | null> {
  return invoke<RegistryError | null>("get_registry_error");
}

/**
 * §7 `get_port_status` (added 2026-08-10, plan 041) — the §11 Ports panel's one snapshot read.
 * Never rejects for an unidentifiable owner (see `PortStatus`'s doc comment in `types.ts`); a
 * genuine rejection here is an IPC-level failure, handled by the store like every other action.
 */
export function getPortStatus(): Promise<PortStatus[]> {
  return invoke<PortStatus[]>("get_port_status");
}

/**
 * §7 `free_port` (added 2026-08-10, plan 042) — SPEC.md §9 step 1's one authorised signal to a
 * process Hangar did not spawn. Rejects with a message if any gate fails; Rust re-verifies
 * everything immediately before signalling, so a rejection here means nothing was touched. `void`
 * on success too — whether the port is still held afterwards is read back via `getPortStatus`,
 * never inferred from this promise settling (see `freePortAction` in `store.ts`).
 */
export function freePort(projectId: string, pid: number): Promise<void> {
  return invoke<void>("free_port", { projectId, pid });
}

/**
 * §7 `find_free_port` (added 2026-08-10, plan 043) — §10 step 4's "Choose for me". Walks upward
 * from `from`, skipping `exclude` (other registered projects' pinned ports) and anything currently
 * accepting a connection. `null`, never `from`, when the walk is exhausted — this alone does not
 * touch the command field; §10 step 4 requires the caller to rewrite the port token too.
 */
export function findFreePort(from: number, exclude: number[]): Promise<number | null> {
  return invoke<number | null>("find_free_port", { from, exclude });
}

/**
 * SPEC.md §11 "Doctor" (added 2026-08-11, plan 057) — one preflight report per registered project,
 * snapshot at call time. Called on panel open and on Refresh only: it never polls, and nothing on
 * the startup path calls it.
 *
 * Never rejects for a project-level problem — a missing folder, an unreadable `.env` or an
 * unhashable lockfile all come back as *findings*, because §7 turns every rejection into a toast
 * and a toast per project on open would be intolerable.
 */
export function getPreflight(): Promise<PreflightReport[]> {
  return invoke<PreflightReport[]>("get_preflight");
}

/**
 * SPEC.md §11 "Launch line" (added 2026-08-11, plan 060) — one local version-control row per
 * registered project, snapshot at call time. This is the one snapshot read that DOES run when the
 * window opens; it is cheap and local by construction (§3, §11):
 *
 * - **No network.** One `git status` against local refs — never `fetch`, never `ls-remote`. A hung
 *   DNS lookup can therefore never become a hung launch.
 * - **No write.** Not a push, pull, commit or stash; the read even passes `--no-optional-locks` so
 *   it will not take git's index lock.
 *
 * Never rejects for a project-level problem: git missing, a timeout and a non-zero exit all come
 * back as `state: "unavailable"` rows, which the line renders as "not checked" — never as silence,
 * and never as a toast.
 */
export function getVcsStatus(): Promise<VcsStatus[]> {
  return invoke<VcsStatus[]>("get_vcs_status");
}

/**
 * SPEC.md §11 "Build freshness" (added 2026-08-11, plan 063) — whether the `.app` on disk is newer
 * than the process answering this call. Two `stat`s and a compile-time constant:
 *
 * - **No network.** Not an update check, not a version server — §3 bans auto-update outright.
 * - **No action.** The payload has no field that could carry one; the line it feeds is text.
 *
 * Never rejects: a missing bundle path, an unreadable executable, an app launched from somewhere
 * unexpected and every non-macOS platform all come back as `newerBuildInstalled: false`. It runs
 * when the window opens, where §7's "every rejection is a toast" would mean a toast on launch.
 */
export function getBuildFreshness(): Promise<BuildFreshness> {
  return invoke<BuildFreshness>("get_build_freshness");
}

/**
 * SPEC.md §18 / plan 053 `get_github_status` — the Inbox panel's one status read. Never rejects
 * for a connection problem: offline/rate-limited/invalid/keychain-denied are all `Ok` values on
 * `GithubStatus.state` (§11: "none of them is a toast, and none is an error"). Reads the OS
 * keychain lazily, at most once per session — never on startup, never before the grid renders.
 */
export function getGithubStatus(): Promise<GithubStatus> {
  return invoke<GithubStatus>("get_github_status");
}

/**
 * SPEC.md §18 / plan 053 `set_github_token` — validates the token against GitHub BEFORE it is
 * ever written to the keychain. Resolves to the resulting `GithubStatus`, including every
 * failure case; only a genuinely unexpected internal failure rejects the promise.
 */
export function setGithubToken(token: string): Promise<GithubStatus> {
  return invoke<GithubStatus>("set_github_token", { token });
}

/**
 * SPEC.md §18 `remove_github_token` — "one obvious action, and must leave no residue." Rejects
 * if the keychain itself refused the delete.
 */
export function removeGithubToken(): Promise<void> {
  return invoke<void>("remove_github_token");
}
