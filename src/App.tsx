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
import BuildFreshnessLine from "./components/BuildFreshnessLine";
import DoctorPanel from "./components/DoctorPanel";
import InboxPanel from "./components/InboxPanel";
import LaunchLine from "./components/LaunchLine";
import LogPanel from "./components/LogPanel";
import MoveToFolderDialog from "./components/MoveToFolderDialog";
import NotesPanel from "./components/NotesPanel";
import PortsPanel from "./components/PortsPanel";
import ProjectGrid from "./components/ProjectGrid";
import SettingsDialog from "./components/SettingsDialog";
import { lastSessionCluster } from "./session";
import {
  loadRegistry,
  openAddDialog,
  openDoctor,
  openInbox,
  openLogs,
  openPorts,
  openSettingsDialog,
  refreshBuildFreshness,
  refreshRegistryQuietly,
  refreshVcs,
  runningCount,
  setSearch,
  startProject,
  setToast,
  useHangarStore,
  visibleProjects,
} from "./store";
import type { ToastTone } from "./store";
import { relativeTime } from "./status";
import type { ProjectView, Status } from "./types";

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
 *
 * Plan 047: `run_project` returns a plain `Result<(), String>` (§7 is frozen), so the frontend
 * cannot tell a port refusal from any other failure kind by parsing `message` — that would be a
 * regex tied to `run.rs`'s wording. Instead the Ports button is shown whenever `projectId`
 * resolves and that project's status is `stopped`/`crashed`, i.e. the Run did not take. Honest
 * for every failed Run, not just a collision — same rule as the "dangling id" guard above.
 */
function Toast({
  message,
  tone = "error",
  projectId,
  projectName,
  projectStatus,
}: {
  message: string;
  tone?: ToastTone;
  projectId?: string;
  projectName?: string;
  projectStatus?: Status;
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
      {projectId && (projectStatus === "stopped" || projectStatus === "crashed") && (
        <button
          type="button"
          onClick={() => {
            void openPorts();
            setToast(null);
          }}
          className="shrink-0 text-xs text-muted underline-offset-2 transition-colors hover:text-text hover:underline"
        >
          Ports
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

/**
 * §11 "Resume last session" (plan 051). The caller only mounts this when nothing is non-`stopped`
 * and the search box is empty; this component itself covers the third condition — cluster
 * non-empty — by rendering nothing when `lastSessionCluster` finds no group.
 *
 * Starting is N sequential `startProject` calls, `for…of` with `await`, never `Promise.all`:
 * §9's per-path mutex serialises same-repo starts anyway, and sequential keeps a second failure
 * from overwriting the single toast slot.
 */
function ResumeLine({ projects }: { projects: ProjectView[] }) {
  const clusterIds = lastSessionCluster(projects);
  if (clusterIds.length === 0) return null;
  const clusterProjects = projects.filter((p) => clusterIds.includes(p.id));
  const latestMs = clusterProjects.reduce((max, p) => {
    const at = p.lastRunAt ? Date.parse(p.lastRunAt) : NaN;
    return !Number.isNaN(at) && at > max ? at : max;
  }, 0);
  const names = clusterProjects.map((p) => p.name);
  const shown = names.slice(0, 3).join(" · ");
  const overflow = names.length > 3 ? ` · +${names.length - 3}` : "";
  const label =
    names.length === 1 ? "Start" : names.length === 2 ? "Start both" : `Start all ${names.length}`;

  const handleStart = async () => {
    for (const id of clusterIds) {
      await startProject(id);
    }
  };

  return (
    <div className="mb-4 flex items-center justify-between rounded-md border border-white/10 bg-white/[0.02] px-4 py-2.5 text-sm">
      <p className="min-w-0 truncate text-muted">
        Last session, {relativeTime(new Date(latestMs).toISOString())} · {shown}
        {overflow}
      </p>
      <button
        type="button"
        onClick={() => void handleStart()}
        className="ml-4 shrink-0 rounded-md border border-white/10 px-3 py-1 text-xs text-text transition-colors hover:bg-white/5"
      >
        {label}
      </button>
    </div>
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
    inboxOpen,
    doctorOpen,
  } = useHangarStore();
  const toastProject = projects.find((p) => p.id === toastProjectId);
  const toastProjectName = toastProject?.name;

  const contentRef = useRef<HTMLDivElement | null>(null);
  // §11's aria-modal on each overlay is a promise the DOM doesn't keep by itself (plan 039) —
  // `inert` on the header+main wrapper below is the actual enforcement. Same fields as the
  // folder band's Esc guard in ProjectGrid.tsx (plan 041 adds `portsOpen` to both; SPEC.md §18 /
  // plan 053 adds `inboxOpen` to both; SPEC.md §11 Doctor / plan 057 adds `doctorOpen` to both —
  // a sixth over-the-grid surface must be in both, or one Esc fires two state changes).
  const overlayOpen = Boolean(
    dialog || openLogsFor || notesFor || portsOpen || inboxOpen || doctorOpen,
  );

  useEffect(() => {
    // SPEC.md §11 "Launch line" (plan 060): the registry first, then the git snapshot — sequential
    // and never awaited by the render, so the grid paints on the registry alone and the line
    // appears underneath it a moment later. It is safe on this path only because the read is local
    // (`vcs.rs`: one `git status`, no network of any kind); the Doctor's report, which may go to
    // the network, is still forbidden here and is still only ever called by its own panel.
    void loadRegistry().then(() => refreshVcs());
  }, []);

  // SPEC.md §11 "Build freshness" (plan 063): whether the `.app` on disk is newer than this
  // process. Separate from the effect above and NOT chained behind the registry — it is two `stat`s
  // in Rust, it needs no project data, and it must not wait on a git snapshot to say that the thing
  // rendering the git snapshot is out of date.
  useEffect(() => {
    void refreshBuildFreshness();
  }, []);

  // SPEC.md §5: pathExists must refresh "at startup, on registry change, and when Run is
  // clicked". Window focus is the natural "user came back from the browser" moment for a folder
  // that moved while Hangar sat in the background — no timer, so this never polls. Plan 038: the
  // quiet refresh, not loadRegistry — coming back from the browser is the single most frequent
  // event in the app, and blanking the whole grid to re-stat three paths is the worst trade.
  useEffect(() => {
    const onFocus = () => {
      void refreshRegistryQuietly();
      // Plan 063: the same "user came back" moment, and the ONLY moment this feature can work —
      // `npm run install:app` finishes in a terminal, the user clicks back to this window, and the
      // bundle on disk is now newer than the process drawing it. Read once at launch it would only
      // ever compare a build against itself. One `stat`, no spawn, no network: the cost that made
      // plan 060 refuse this path (N git children) is not present here.
      void refreshBuildFreshness();
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
      {/* Plan 046 step 4: the one shared wrapper for both the header row and <main>'s content —
          already the `inert` boundary (plan 039), so capping width here instead of adding a new
          div keeps that guard intact. Capping only the grid was rejected: it would move every
          Run button inward while "Add project" stayed pinned to the far edge.

          LEFT-ALIGNED, not centred, and capped at 120rem rather than 80rem (fixed 2026-08-10,
          same day 046 shipped). `mx-auto max-w-[80rem]` centred the whole app, so at fullscreen
          on a 1512-logical display the "Hangar" title sat 200px+ from the left edge with a band
          of empty background beside it — the maintainer's report, and correct: a launcher's mark
          belongs in the top-left corner, not floating in the middle of the titlebar row.
          `mr-auto` keeps that corner while the cap still stops a very wide monitor sprawling.
          At 1512 and 1728 logical the 120rem cap is a no-op (6 and 7 columns, zero waste); it
          only engages past ~1984px. */}
      <div ref={contentRef} className="mr-auto flex w-full max-w-[120rem] flex-1 flex-col">
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
          {/* SPEC.md §11 Doctor (plan 057): a quiet button beside Ports, hidden when there is
              nothing registered to check. Opening it is the ONLY thing that runs preflight —
              nothing here or in the mount effect above calls it before the grid renders. */}
          {projects.length > 0 && (
            <button
              type="button"
              onClick={() => void openDoctor()}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              Doctor
            </button>
          )}
          {/* SPEC.md §18 / plan 053: a quiet Inbox button — no unread count yet (that needs the
              local cache slice 2 builds; §11 forbids it making a network/keychain call itself). */}
          {projects.length > 0 && (
            <button
              type="button"
              onClick={() => void openInbox()}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              Inbox
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

        {/* §11 "Build freshness" (plan 063): first of the three lines, because it is the only one
            about the app itself — if the window is running an old build, every other thing on this
            screen may be answering for code that is no longer what is installed. Like the launch
            line it has no render condition here: silence is decided inside the component, which
            returns null unless Rust said the bundle on disk is newer. Text only — no restart, no
            kill, no update download (§3, §8). */}
        <BuildFreshnessLine />

        {/* §11 "Launch line" (plan 060): above the resume line, because "you have thirty commits
            on one laptop" outranks "want to start yesterday's set?". It has no render condition of
            its own here — silence is decided inside the component, which returns null whenever
            there is nothing to report. It is NOT hidden under an active search: a search filters
            the grid, and hiding what needs attention because the user typed three letters would
            lose the one fact this element exists to deliver. */}
        <LaunchLine projects={projects} />

        {/* §11 "Resume last session" (plan 051): the third condition — cluster non-empty — is
            covered inside ResumeLine itself, which renders nothing when there is no group. */}
        {projects.every((p) => p.status === "stopped") && search.trim() === "" && (
          <ResumeLine projects={projects} />
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
      <InboxPanel />
      <DoctorPanel />
      <AddEditDialog />
      <MoveToFolderDialog />
      <SettingsDialog />
      {toast && (
        <Toast
          message={toast}
          tone={toastTone}
          projectId={toastProjectId ?? undefined}
          projectName={toastProjectName}
          projectStatus={toastProject?.status}
        />
      )}
    </div>
  );
}

export default App;
