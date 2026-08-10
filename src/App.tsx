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
import { useEffect, useRef } from "react";
import AddEditDialog from "./components/AddEditDialog";
import LogPanel from "./components/LogPanel";
import MoveToFolderDialog from "./components/MoveToFolderDialog";
import NotesPanel from "./components/NotesPanel";
import PortsPanel from "./components/PortsPanel";
import ProjectGrid from "./components/ProjectGrid";
import SettingsDialog from "./components/SettingsDialog";
import {
  loadRegistry,
  openAddDialog,
  openLogs,
  openPorts,
  openSettingsDialog,
  refreshRegistryQuietly,
  runningCount,
  setSearch,
  setToast,
  useHangarStore,
  visibleProjects,
} from "./store";
import type { ToastTone } from "./store";

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

/**
 * §7: command errors surface as toasts ("errors always say what happened and what to do next").
 * `tone` defaults to the original error styling — plan 029 adds `"neutral"` for the
 * move-to-folder confirmation, which is an announcement, not something gone wrong.
 *
 * Plan 034: `projectName` is looked up by the caller from `toastProjectId` — `undefined` when
 * there is no id, or the id no longer resolves (project removed meanwhile). Only then does the
 * message get a "<name> — " prefix and a Show logs button; a dangling id must never render a
 * stale name.
 */
function Toast({
  message,
  tone = "error",
  projectId,
  projectName,
}: {
  message: string;
  tone?: ToastTone;
  projectId?: string;
  projectName?: string;
}) {
  return (
    <div
      role="alert"
      className={`hangar-fade-in fixed bottom-6 left-1/2 z-30 flex max-w-[36rem] -translate-x-1/2 items-start gap-4 rounded-md border bg-surface px-4 py-3 text-sm text-text shadow-lg ${
        tone === "neutral" ? "border-white/10" : "border-status-danger/40"
      }`}
    >
      <span className="min-w-0">
        {projectName ? <span className="font-medium">{projectName} — </span> : null}
        {message}
      </span>
      {projectId && projectName && (
        <button
          type="button"
          onClick={() => {
            void openLogs(projectId);
            setToast(null);
          }}
          className="shrink-0 text-xs text-muted underline-offset-2 transition-colors hover:text-text hover:underline"
        >
          Show logs
        </button>
      )}
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
  const {
    projects,
    registryError,
    loading,
    loadError,
    toast,
    toastTone,
    toastProjectId,
    search,
    openLogsFor,
    notesFor,
    dialog,
    portsOpen,
  } = useHangarStore();
  const toastProjectName = projects.find((p) => p.id === toastProjectId)?.name;

  const contentRef = useRef<HTMLDivElement | null>(null);
  // §11's aria-modal on each overlay is a promise the DOM doesn't keep by itself (plan 039) —
  // `inert` on the header+main wrapper below is the actual enforcement. Same four fields as
  // the folder band's Esc guard in ProjectGrid.tsx (plan 041 adds `portsOpen` to both).
  const overlayOpen = Boolean(dialog || openLogsFor || notesFor || portsOpen);

  useEffect(() => {
    void loadRegistry();
  }, []);

  // SPEC.md §5: pathExists must refresh "at startup, on registry change, and when Run is
  // clicked". Window focus is the natural "user came back from the browser" moment for a folder
  // that moved while Hangar sat in the background — no timer, so this never polls. Plan 038: the
  // quiet refresh, not loadRegistry — coming back from the browser is the single most frequent
  // event in the app, and blanking the whole grid to re-stat three paths is the worst trade.
  useEffect(() => {
    const onFocus = () => {
      void refreshRegistryQuietly();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  // React 18 has no typed `inert` JSX prop, so this sets the DOM attribute imperatively via the
  // ref instead of `{...{ inert: "" }}` or `@ts-expect-error` — `setAttribute`/`removeAttribute`
  // are plain, fully-typed `HTMLElement` methods. The five overlays below render as siblings
  // *after* this ref's wrapper, never inside it, so this can never make an open dialog inert.
  useEffect(() => {
    const node = contentRef.current;
    if (!node) return;
    if (overlayOpen) node.setAttribute("inert", "");
    else node.removeAttribute("inert");
  }, [overlayOpen]);

  return (
    <div className="flex min-h-full flex-col bg-bg text-text">
      <div ref={contentRef} className="flex flex-1 flex-col">
      <header className="flex items-center justify-between border-b border-white/5 px-8 py-5">
        <div className="flex items-center gap-3">
          {/* Plan 046 step 3: text-xl -> text-2xl (20 -> 24px). text-2xl's 32px line box is still
              under the 38px search input's height, so the 79px header is unchanged. Today the
              largest type in the app is the Add tile's "+" glyph — this gives the page a top. */}
          <h1 className="font-display text-2xl font-bold tracking-tight text-text">Hangar</h1>
          {/* SPEC.md §11: a quiet aggregate — how many projects are running right now. */}
          {runningCount(projects) > 0 && (
            <span className="text-sm text-muted">{runningCount(projects)} running</span>
          )}
        </div>
        <div className="flex items-center gap-2">
          {projects.length > 0 && <SearchInput value={search} />}
          {/* §11 Ports panel (plan 041): quiet, hidden when there is nothing registered to show. */}
          {projects.length > 0 && (
            <button
              type="button"
              onClick={() => void openPorts()}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              Ports
            </button>
          )}
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
          <ProjectGrid projects={visibleProjects(projects, search)} search={search} />
        )}
      </main>
      </div>

      <LogPanel />
      <NotesPanel />
      <PortsPanel />
      <AddEditDialog />
      <MoveToFolderDialog />
      <SettingsDialog />
      {toast && (
        <Toast
          message={toast}
          tone={toastTone}
          projectId={toastProjectId ?? undefined}
          projectName={toastProjectName}
        />
      )}
    </div>
  );
}

export default App;
