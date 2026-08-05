/**
 * The only file that calls `invoke()` (SPEC.md §7, §13). Components import from here.
 *
 * M1 wired the read-only slice of the frozen API; M2 added `run_project`, `get_log_buffer` and
 * `clear_log_buffer`; M3 adds `stop_project`. The rest (`add_project`, `update_project`,
 * `remove_project`, `read_package_json`, `open_in_editor`, `open_in_browser`) are added by
 * plans 004–005 under exactly those names.
 */
import { invoke } from "@tauri-apps/api/core";
import type { LogLine, ProjectView, RegistryError, Settings } from "./types";

export function getProjects(): Promise<ProjectView[]> {
  return invoke<ProjectView[]>("get_projects");
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
