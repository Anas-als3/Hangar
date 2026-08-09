/**
 * App shell — SPEC.md §11.
 *
 * Header, the corrupt-registry banner, the grid, the empty state, and (M2) the log slide-over.
 * M5 (this plan) wires the Add buttons and the settings gear to the AddEditDialog/SettingsDialog.
 *
 * Status arrives from one `get_projects()` call plus the `status-changed` event — never polling
 * (§7). Both event listeners are registered once at startup in `src/main.tsx`, not here and
 * certainly not in `LogPanel`.
 */
import { useEffect } from "react";
import AddEditDialog from "./components/AddEditDialog";
import LogPanel from "./components/LogPanel";
import NotesPanel from "./components/NotesPanel";
import ProjectGrid from "./components/ProjectGrid";
import SettingsDialog from "./components/SettingsDialog";
import {
  loadRegistry,
  openAddDialog,
  openSettingsDialog,
  setSearch,
  setToast,
  useHangarStore,
  visibleProjects,
} from "./store";

function CorruptRegistryBanner({
  backupPath,
  error,
}: {
  backupPath: string | null;
  error: string;
}) {
  return (
    <div
      role="alert"
      className="mb-6 rounded-md border border-status-danger/40 bg-status-danger/10 p-4 text-sm"
    >
      <p className="font-medium text-status-danger">
        projects.json could not be read, so Hangar started with an empty library.
      </p>
      <p className="mt-2 text-muted">
        Nothing was overwritten.{" "}
        {backupPath ? (
          <>
            Your original file was moved to{" "}
            <span className="font-mono text-text">{backupPath}</span>. Fix the JSON there and copy
            it back over projects.json, then restart Hangar.
          </>
        ) : (
          <>The original file was left in place. Fix it, then restart Hangar.</>
        )}
      </p>
      <p className="mt-2 font-mono text-xs text-muted">{error}</p>
    </div>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-5 py-24 text-center">
      <p className="font-display text-xl text-text">No projects yet. Add your first one.</p>
      <button
        type="button"
        onClick={openAddDialog}
        className="rounded-md bg-accent px-5 py-2 text-sm font-semibold text-bg transition-opacity hover:opacity-90"
      >
        Add project
      </button>
    </div>
  );
}

/** §7: command errors surface as toasts ("errors always say what happened and what to do next"). */
function Toast({ message }: { message: string }) {
  return (
    <div
      role="alert"
      className="hangar-fade-in fixed bottom-6 left-1/2 z-30 flex max-w-[36rem] -translate-x-1/2 items-start gap-4 rounded-md border border-status-danger/40 bg-surface px-4 py-3 text-sm text-text shadow-lg"
    >
      <span className="min-w-0">{message}</span>
      <button
        type="button"
        aria-label="Dismiss"
        onClick={() => setToast(null)}
        className="shrink-0 text-muted transition-colors hover:text-text"
      >
        <span aria-hidden="true">✕</span>
      </button>
    </div>
  );
}

/** Header search box (plan 017). Hidden by the caller when the registry is empty. */
function SearchInput({ value }: { value: string }) {
  return (
    <input
      type="search"
      aria-label="Search projects"
      placeholder="Search projects"
      value={value}
      onChange={(e) => setSearch(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Escape") setSearch("");
      }}
      className="rounded-md border border-white/10 bg-bg px-3 py-2 text-sm text-text outline-none focus:border-accent"
    />
  );
}

function App() {
  const { projects, registryError, loading, loadError, toast, search } = useHangarStore();

  useEffect(() => {
    void loadRegistry();
  }, []);

  return (
    <div className="flex min-h-full flex-col bg-bg text-text">
      <header className="flex items-center justify-between border-b border-white/5 px-8 py-5">
        <h1 className="font-display text-xl font-bold tracking-tight text-text">Hangar</h1>
        <div className="flex items-center gap-2">
          {projects.length > 0 && <SearchInput value={search} />}
          <button
            type="button"
            onClick={openAddDialog}
            className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-text transition-colors hover:bg-white/5"
          >
            Add project
          </button>
          <button
            type="button"
            aria-label="Settings"
            onClick={openSettingsDialog}
            className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
          >
            <span aria-hidden="true">⚙</span>
          </button>
        </div>
      </header>

      <main className="flex flex-1 flex-col px-8 py-6">
        {registryError && (
          <CorruptRegistryBanner
            backupPath={registryError.backupPath}
            error={registryError.error}
          />
        )}

        {loadError && (
          <div
            role="alert"
            className="mb-6 rounded-md border border-status-danger/40 bg-status-danger/10 p-4 text-sm text-status-danger"
          >
            Could not load the project registry: {loadError}
          </div>
        )}

        {loading ? (
          <p className="text-sm text-muted">Loading…</p>
        ) : projects.length === 0 ? (
          <EmptyState />
        ) : visibleProjects(projects, search).length === 0 ? (
          <p className="text-sm text-muted">No projects match &quot;{search.trim()}&quot;.</p>
        ) : (
          <ProjectGrid projects={visibleProjects(projects, search)} />
        )}
      </main>

      <LogPanel />
      <NotesPanel />
      <AddEditDialog />
      <SettingsDialog />
      {toast && <Toast message={toast} />}
    </div>
  );
}

export default App;
