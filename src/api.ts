/**
 * The only file that calls `invoke()` (SPEC.md §7, §13). Components import from here.
 *
 * M1 wires the read-only slice of the frozen API. The remaining commands
 * (`add_project`, `update_project`, `remove_project`, `run_project`, `stop_project`,
 * `get_log_buffer`, `clear_log_buffer`, `read_package_json`, `open_in_editor`,
 * `open_in_browser`) are added by plans 002–005 under exactly those names.
 */
import { invoke } from "@tauri-apps/api/core";
import type { ProjectView, RegistryError, Settings } from "./types";

export function getProjects(): Promise<ProjectView[]> {
  return invoke<ProjectView[]>("get_projects");
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
