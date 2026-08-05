/**
 * Global store for project status and logs (SPEC.md §13).
 *
 * At M1 it holds the registry snapshot from the single `get_projects()` call plus the
 * corrupt-registry banner. Plan 002 registers the two §7 event listeners — `status-changed`
 * and `log-lines` — ONCE at app startup and feeds them into `setStatus` / the log slice here;
 * they must never be registered inside the log-panel component.
 */
import { useSyncExternalStore } from "react";
import { getProjects, getRegistryError } from "./api";
import type { ProjectView, RegistryError } from "./types";

export interface HangarState {
  projects: ProjectView[];
  registryError: RegistryError | null;
  loading: boolean;
  loadError: string | null;
}

let state: HangarState = {
  projects: [],
  registryError: null,
  loading: true,
  loadError: null,
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
    setState({
      loading: false,
      loadError: err instanceof Error ? err.message : String(err),
    });
  }
}
