/**
 * App shell — SPEC.md §11.
 *
 * M1 scope: header, the corrupt-registry banner, the grid, and the empty state.
 * The Add button and the settings gear open nothing yet (plan 005); the log slide-over is
 * plan 002. Status arrives from one `get_projects()` call — the two §7 event listeners are
 * registered once at startup by plan 002.
 */
import { useEffect } from "react";
import ProjectGrid from "./components/ProjectGrid";
import { loadRegistry, useHangarStore } from "./store";

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
        className="rounded-md bg-accent px-5 py-2 text-sm font-semibold text-bg transition-opacity hover:opacity-90"
      >
        Add project
      </button>
    </div>
  );
}

function App() {
  const { projects, registryError, loading, loadError } = useHangarStore();

  useEffect(() => {
    void loadRegistry();
  }, []);

  return (
    <div className="flex min-h-full flex-col bg-bg text-text">
      <header className="flex items-center justify-between border-b border-white/5 px-8 py-5">
        <h1 className="font-display text-xl font-bold tracking-tight text-text">Hangar</h1>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-text transition-colors hover:bg-white/5"
          >
            Add project
          </button>
          <button
            type="button"
            aria-label="Settings"
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
        ) : (
          <ProjectGrid projects={projects} />
        )}
      </main>
    </div>
  );
}

export default App;
