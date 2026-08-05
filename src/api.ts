/**
 * The only file that calls `invoke()` (SPEC.md §7, §13). Components import from here.
 *
 * M1 wired the read-only slice of the frozen API; M2 adds `run_project`, `get_log_buffer`
 * and `clear_log_buffer`. The rest (`add_project`, `update_project`, `remove_project`,
 * `stop_project`, `read_package_json`, `open_in_editor`, `open_in_browser`) are added by
 * plans 003–005 under exactly those names.
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
