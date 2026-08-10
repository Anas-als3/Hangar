//! All `#[tauri::command]` functions (SPEC.md §7 — the API is FROZEN).
//!
//! M2 adds the spawn/log slice: `run_project`, `get_log_buffer`, `clear_log_buffer`.
//! The rest (`add_project`, `stop_project`, editor/browser …) arrive with plans 003–005 and must
//! keep the exact names and shapes §7 gives them. `stop_project` is plan 003: there is deliberately
//! no stub that "sort of" stops.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

use tauri::{AppHandle, State};
use tokio::sync::Mutex;

use serde::Serialize;

use crate::env_resolve::DevEnvCell;
use crate::process::{self, LogLine, RuntimeMap};
use crate::registry::{self, Project, ProjectView, RegistryError, Settings, Status};

/// SPEC.md §7 `get_port_status` (added 2026-08-10, plan 041 — the Ports panel). One entry per
/// registered project, snapshot at call time. `busy`/`listenerCount`/`holder` are never widened
/// into an error: a lookup that fails or times out yields `busy: true, listenerCount: 0,
/// holder: None` (the "owner unknown" row), because a diagnostic panel that can error on a
/// perfectly normal "couldn't identify the process" case would be worse than the toast it replaces.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortStatus {
    pub project_id: String,
    pub port: u16,
    pub busy: bool,
    /// > 1 → §11 names nobody and offers nothing; the panel says so instead of guessing.
    pub listener_count: usize,
    /// Only `Some` when `listener_count == 1` and the lsof row parsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holder: Option<PortHolder>,
    /// ISO — one timestamp shared by every row in a single `get_port_status` call.
    pub checked_at: String,
}

/// SPEC.md §7 `PortHolder`. `command`/`started_at`/`parent_exited` come from one batched Unix `ps`
/// read (see `commands::get_port_status`); all three stay `None` on Windows, where no equivalent
/// read is implemented yet.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortHolder {
    pub name: String,
    pub pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_exited: Option<bool>,
    /// `false` → `free_port` (plan 042) must never be offered; `None` when the current user's own
    /// identity could not be determined, which must be just as inert as `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_user: Option<bool>,
}

/// Managed state (SPEC.md §4). The mutexes are `tokio::sync::Mutex` — never the blocking std one:
/// kill/wait sequences `.await` while this state is consulted, and a blocking guard may never be
/// held across an `.await`.
pub struct AppState {
    pub config_dir: PathBuf,
    pub projects: Mutex<Vec<Project>>,
    pub settings: Mutex<Settings>,
    /// Set once at startup when `projects.json` could not be parsed; drives the §11 banner.
    pub registry_error: Option<RegistryError>,
    /// Per-project status, log ring buffer, live child and (on Windows) Job Object handle.
    pub runtime: Mutex<RuntimeMap>,
    /// The §8 login-shell environment, resolved once and shared by every child.
    pub dev_env: DevEnvCell,
    /// SPEC.md §8 quit interception: set once every tree has been killed. Both interception paths
    /// check it, so the `app_handle.exit(0)` that follows cleanup passes straight through instead of
    /// bouncing off the very guard that triggered the cleanup.
    pub cleanup_done: AtomicBool,
    /// A confirm dialog is already open (or the kill is already running). Without it, holding Cmd+Q
    /// or clicking the close button twice stacks dialogs and starts two stop-everything passes.
    pub quit_in_flight: AtomicBool,
}

impl AppState {
    pub fn new(
        config_dir: PathBuf,
        projects: Vec<Project>,
        settings: Settings,
        registry_error: Option<RegistryError>,
    ) -> Self {
        Self {
            config_dir,
            projects: Mutex::new(projects),
            settings: Mutex::new(settings),
            registry_error,
            runtime: Mutex::new(RuntimeMap::new()),
            dev_env: DevEnvCell::default(),
            cleanup_done: AtomicBool::new(false),
            quit_in_flight: AtomicBool::new(false),
        }
    }
}

/// Derived, never persisted (SPEC.md §5). Status comes from the runtime map — a project that has
/// never run is `stopped`.
fn to_view(project: &Project, runtime: &RuntimeMap) -> ProjectView {
    ProjectView {
        project: project.clone(),
        status: runtime
            .get(&project.id)
            .map(|r| r.status)
            .unwrap_or(Status::Stopped),
        path_exists: Path::new(&project.path).exists(),
    }
}

#[tauri::command]
pub async fn get_projects(state: State<'_, AppState>) -> Result<Vec<ProjectView>, String> {
    let projects = state.projects.lock().await;
    let runtime = state.runtime.lock().await;
    // Array order is the display order — no sorting, ever (SPEC.md §11).
    Ok(projects.iter().map(|p| to_view(p, &runtime)).collect())
}

/// SPEC.md §7 `add_project` / §10 steps 1-6. The one place a fresh registry entry is created —
/// the duplicate-port rejection (§10 step 5) is enforced here, in Rust, so a frontend that skipped
/// its own check still cannot register a second project on a port already in use.
#[tauri::command]
pub async fn add_project(
    input: registry::NewProject,
    state: State<'_, AppState>,
) -> Result<ProjectView, String> {
    registry::validate_ready_timeout_sec(input.ready_timeout_sec)?;

    let mut projects = state.projects.lock().await;

    if let Some(owner) = registry::port_conflict(&projects, input.port, None) {
        return Err(format!(
            "Port {} is already used by {}.",
            input.port, owner.name
        ));
    }

    let project = Project {
        id: registry::generate_id(),
        name: input.name,
        path: input.path,
        command: input.command,
        port: input.port,
        url: input.url,
        update_on_run: input.update_on_run,
        ready_timeout_sec: input.ready_timeout_sec,
        last_lockfile_hash: None,
        last_run_at: None,
        notes: input.notes,
        // SPEC.md §5/§7 (plan 023): the Add dialog's `read_package_json` call already detected
        // this — carried straight through the wire, never recomputed here.
        stack: input.stack,
        // SPEC.md §5 (folders, 2026-08-10): a new project may already be filed into a folder
        // from the Add dialog — carried straight through, same as `notes`/`stack`.
        folder_id: input.folder_id,
        folder_name: input.folder_name,
    };

    projects.push(project.clone());
    registry::save_projects(&state.config_dir, &projects)?;

    // A brand-new project has never run, so `stopped` (the runtime map's default for an absent
    // entry) is the truthful status — no need to touch the runtime lock to prove it.
    let runtime = state.runtime.lock().await;
    Ok(to_view(&project, &runtime))
}

/// SPEC.md §6 mutation guard vs. the run-inert field set (plan 028 revision): `guard_mutation`
/// exists because mutating a *running* project can break the run itself — a changed `port` breaks
/// Stop's port verification, a changed `path`/`command` breaks the kill path. §6's amended bullet
/// names the run-inert set as exactly `notes`, `folderId` and `folderName`: none of them is read
/// by §8's spawn/kill paths or §9's run sequence, so a change confined to them provably cannot
/// affect a running project — it is exempt from the guard.
///
/// Deliberately not a hand-enumerated field list: normalising the run-inert set out of both sides
/// and comparing the rest with the derived `PartialEq` means any other field — including ones a
/// future plan adds — is covered by the guard automatically, with nothing to remember to update
/// here.
///
/// Also normalises out the **app-owned** set (SPEC.md §6's app-owned bullet, added 2026-08-10):
/// `last_run_at` and `last_lockfile_hash`. The backend writes both without telling the frontend
/// (`run.rs`'s per-Run save persists `last_run_at`; the install phase persists
/// `last_lockfile_hash`), and `status-changed` carries only `status`, so the caller's payload is
/// stale in these two fields for the entire updating/installing/starting window — exactly when a
/// run-inert save (a note, a folder move) is most likely to happen. Leaving them in the comparison
/// made the run-inert exemption unreachable during that window. This does not widen what a caller
/// can change: `update_project` (see `merge_run_inert_fields` and step 3 below) preserves both
/// fields from the stored record on every write, guarded or not, so a caller's value for them is
/// never actually written regardless of what this comparison decides.
fn is_run_inert_change(stored: &Project, incoming: &Project) -> bool {
    let mut stored = stored.clone();
    let mut incoming = incoming.clone();
    stored.notes = None;
    incoming.notes = None;
    stored.folder_id = None;
    incoming.folder_id = None;
    stored.folder_name = None;
    incoming.folder_name = None;
    stored.last_run_at = None;
    incoming.last_run_at = None;
    stored.last_lockfile_hash = None;
    incoming.last_lockfile_hash = None;
    // `stack` stays writable (see `stack_is_unchanged_ignoring_timestamp`'s doc comment) — it is
    // only normalised out here, field by field, when the helper says the difference is nothing
    // but a re-stamped `detected_at`. A genuine stack change is left in place so it still guards.
    if stack_is_unchanged_ignoring_timestamp(&stored.stack, &incoming.stack) {
        stored.stack = None;
        incoming.stack = None;
    }
    stored == incoming
}

/// SPEC.md §6 (added 2026-08-10): `stack` must stay writable from the payload — the Edit dialog
/// re-detects it on every open (plan 025) — so unlike `lastRunAt`/`lastLockfileHash` it cannot
/// join the app-owned set. But `registry.rs`'s `detect_stack` re-stamps `detected_at` on every Run
/// (registry.rs:559) just as it does on Edit, so a stale frontend payload differs from stored in
/// `stack.detectedAt` alone during the same window `lastRunAt` goes stale — the same failure, one
/// field over. `stack` counts as unchanged for the run-inert comparison when the incoming value is
/// `None`, or when it differs from stored only in `detected_at`; a caller that genuinely changed
/// the detected framework or libraries is still a guarded change.
fn stack_is_unchanged_ignoring_timestamp(
    stored: &Option<registry::ProjectStack>,
    incoming: &Option<registry::ProjectStack>,
) -> bool {
    match (stored, incoming) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(stored), Some(incoming)) => {
            stored.framework == incoming.framework && stored.libraries == incoming.libraries
        }
    }
}

/// SPEC.md §7 `update_project` / §6 / §10 step 7: "Remove/Edit while status ∉ {stopped, crashed}
/// first shows a confirm ... confirming runs the full §8 kill and waits for verification before
/// removing/saving." The frontend's confirm dialog already called `stop_project` and awaited its
/// verified death before calling this — `guard_mutation` here is what makes that real: a frontend
/// that skipped the confirm (or raced it) cannot save over a project whose tree is still alive.
///
/// Exception: a run-inert change (see `is_run_inert_change`, SPEC.md §6's amended bullet) skips
/// the guard, so the Notes slide-over can autosave and a card can be filed into a folder while a
/// project is running — the whole point of per-project notes is to record something while or
/// right after testing it (SPEC.md §11), and folders would be useless if filing one away required
/// stopping it first.
///
/// Takes `is_run_inert` as a precomputed bool rather than calling `is_run_inert_change` itself:
/// `update_project` (SPEC.md §6's merge-not-replace bullet, plan 028) needs the same boolean again
/// afterward to decide whether to merge or replace the stored record, and the predicate must run
/// exactly once so the guard decision and the write decision can never disagree.
///
/// Pulled out of `update_project` as plain data in, `Result` out — no `State`/`AppHandle` — so
/// the decision itself is unit-testable without standing up a Tauri app in the test harness.
fn guard_update(is_run_inert: bool, stored: &Project, status: Status) -> Result<(), String> {
    if is_run_inert {
        return Ok(());
    }
    crate::run::guard_mutation(status, &stored.name)
}

/// SPEC.md §6 (added 2026-08-10): "a run-inert update writes only the run-inert fields into the
/// stored record — it must never replace the whole record from the caller's payload." `run.rs`
/// persists `last_run_at` and a freshly detected `stack` on every Run, and the frontend's copy of
/// both goes stale for the whole `updating`→`installing`→`starting` window (the `status-changed`
/// event carries only `status`), so a whole-record write here would silently roll them back.
/// Merges only `notes`/`folder_id`/`folder_name` from `incoming` into `stored`; every other field
/// is left exactly as stored. Plain data in, no return, no `State`/`AppHandle` — unit-testable
/// without a Tauri app, same reasoning as `guard_update`.
fn merge_run_inert_fields(stored: &mut Project, incoming: Project) {
    stored.notes = incoming.notes;
    stored.folder_id = incoming.folder_id;
    stored.folder_name = incoming.folder_name;
}

/// SPEC.md §6's app-owned bullet (added 2026-08-10): "Both fields are preserved from the stored
/// record on every write, guarded or not." The merge branch above already satisfies this — it
/// never touches `last_run_at`/`last_lockfile_hash` at all. This is the replace branch's half: a
/// guarded (non-run-inert) `update_project` call still carries the caller's stale copy of both
/// app-owned fields, and a bare `projects[index] = project` would silently roll them back. Plain
/// data in, data out — unit-testable without a Tauri app, same reasoning as `guard_update`.
fn replace_preserving_app_owned_fields(stored: &Project, incoming: Project) -> Project {
    let mut replaced = incoming;
    replaced.last_run_at = stored.last_run_at.clone();
    replaced.last_lockfile_hash = stored.last_lockfile_hash.clone();
    replaced
}

#[tauri::command]
pub async fn update_project(
    project: Project,
    state: State<'_, AppState>,
) -> Result<ProjectView, String> {
    registry::validate_ready_timeout_sec(project.ready_timeout_sec)?;

    let mut projects = state.projects.lock().await;
    let runtime = state.runtime.lock().await;

    let index = projects
        .iter()
        .position(|p| p.id == project.id)
        .ok_or_else(|| format!("no project with id {}", project.id))?;

    let status = runtime
        .get(&project.id)
        .map(|r| r.status)
        .unwrap_or(Status::Stopped);
    // Computed once, reused below to decide the write shape — never call this a second time.
    let is_run_inert = is_run_inert_change(&projects[index], &project);
    guard_update(is_run_inert, &projects[index], status)?;

    if let Some(owner) = registry::port_conflict(&projects, project.port, Some(&project.id)) {
        return Err(format!(
            "Port {} is already used by {}.",
            project.port, owner.name
        ));
    }

    if is_run_inert {
        merge_run_inert_fields(&mut projects[index], project);
    } else {
        // SPEC.md §6's app-owned bullet: a guarded replace must not roll back `lastRunAt`/
        // `lastLockfileHash` to whatever stale copy the caller's payload carried.
        projects[index] = replace_preserving_app_owned_fields(&projects[index], project);
    }
    registry::save_projects(&state.config_dir, &projects)?;

    // Built from the stored record, not the caller's payload: after a merge the two can differ
    // (`lastRunAt`/`stack`), and the view must reflect what was actually written (SPEC.md §6).
    Ok(to_view(&projects[index], &runtime))
}

/// SPEC.md §7 `remove_project`: "rejected with a message if status ∉ {stopped, crashed}". Same
/// `guard_mutation` as `update_project`, for the same reason — see its doc comment.
#[tauri::command]
pub async fn remove_project(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut projects = state.projects.lock().await;
    let runtime = state.runtime.lock().await;

    let index = projects
        .iter()
        .position(|p| p.id == id)
        .ok_or_else(|| format!("no project with id {id}"))?;

    let status = runtime.get(&id).map(|r| r.status).unwrap_or(Status::Stopped);
    crate::run::guard_mutation(status, &projects[index].name)?;

    projects.remove(index);
    registry::save_projects(&state.config_dir, &projects)?;
    Ok(())
}

/// SPEC.md §7: fire-and-forget from the frontend's point of view — all progress arrives via the
/// `status-changed` and `log-lines` events. The returned error is the toast for a rejected Run
/// (wrong status, missing folder, spawn failure).
#[tauri::command]
pub async fn run_project(id: String, app: AppHandle) -> Result<(), String> {
    crate::run::run_project(&app, &id).await
}

/// SPEC.md §7 `stop_project`. Awaits the whole §8 sequence — kill, reap, verified death, then the
/// port — so the returned `Result` is a truthful answer: `Ok` means the tree is gone, `Err` is the
/// `stop-failed` toast. Status still arrives via `status-changed`; nothing polls.
#[tauri::command]
pub async fn stop_project(id: String, app: AppHandle) -> Result<(), String> {
    crate::run::stop_project(&app, &id).await
}

/// SPEC.md §7 `open_in_browser` — the overflow-menu action. The automatic hand-off on entering
/// `running` (§9 step 6) does not go through here; both call `run::open_in_browser`, which uses the
/// opener plugin **from Rust** (§4: that bypasses the ACL, so no capability entry is needed).
#[tauri::command]
pub async fn open_in_browser(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let project = {
        let projects = state.projects.lock().await;
        projects
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| format!("no project with id {id}"))?
    };
    crate::run::open_in_browser(&app, &project).await
}

/// SPEC.md §7 `open_in_editor` / §10 step 7. Goes through `run::open_in_editor`, which uses the
/// ONE §8 spawn helper — never a bare `Command` (`code` is a `.cmd` shim on Windows).
#[tauri::command]
pub async fn open_in_editor(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let project = {
        let projects = state.projects.lock().await;
        projects
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| format!("no project with id {id}"))?
    };
    crate::run::open_in_editor(&app, &project).await
}

/// SPEC.md §8: Rust owns the buffer; the panel backfills from it on open.
#[tauri::command]
pub async fn get_log_buffer(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<LogLine>, String> {
    let runtime = state.runtime.lock().await;
    Ok(runtime.get(&id).map(|r| r.logs.snapshot()).unwrap_or_default())
}

#[tauri::command]
pub async fn clear_log_buffer(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut runtime = state.runtime.lock().await;
    if let Some(entry) = runtime.get_mut(&id) {
        entry.logs.clear();
    }
    Ok(())
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.settings.lock().await.clone())
}

#[tauri::command]
pub async fn set_settings(s: Settings, state: State<'_, AppState>) -> Result<(), String> {
    let mut settings = state.settings.lock().await;
    registry::save_settings(&state.config_dir, &s)?;
    *settings = s;
    Ok(())
}

/// DEVIATION from SPEC.md §7: the frozen list has no vehicle for the corrupt-registry banner that
/// §4 and §12 both require ("persistent error banner naming the backup file and the parse error").
/// This is an addition, not a rename or reshape of anything in §7.
#[tauri::command]
pub async fn get_registry_error(
    state: State<'_, AppState>,
) -> Result<Option<RegistryError>, String> {
    Ok(state.registry_error.clone())
}

/// SPEC.md §7 `read_package_json` — the Add dialog's script list, package-manager detection and
/// port suggestion (§10 steps 2-4, 6). Never errors: a missing/unparseable `package.json` reads
/// as empty scripts (see `registry::read_package_json`'s doc comment), which is what lets the
/// dialog fall back to manual command + port entry.
#[tauri::command]
pub async fn read_package_json(path: String) -> Result<registry::PackageJsonInfo, String> {
    Ok(registry::read_package_json(Path::new(&path)))
}

/// SPEC.md §7 `get_port_status` (added 2026-08-10, plan 041 — the §11 Ports panel). One entry per
/// registered project, in `projects.json` array order, snapshot at call time — the panel itself
/// never polls (§11: "reads once on open and again only on Refresh").
#[tauri::command]
pub async fn get_port_status(state: State<'_, AppState>) -> Result<Vec<PortStatus>, String> {
    // Lock discipline (plan 041's hard limit): snapshot (id, port) under the lock, THEN drop it,
    // THEN probe — `lsof`/`ps` below are awaits, and §4 forbids holding the async mutex across
    // one. Mirrors `run.rs`'s `_path_guard` shape: the block ends and the guard drops before any
    // lookup runs.
    let snapshot: Vec<(String, u16)> = {
        let projects = state.projects.lock().await;
        projects.iter().map(|p| (p.id.clone(), p.port)).collect()
    };

    let env = state.dev_env.get().await.vars.clone();
    let checked_at = crate::run::iso8601_utc(SystemTime::now());

    // Pass 1: probe every port — one `lsof` per port, never a machine-wide query (§3 bans a
    // network inspector). `busy` travels alongside `listeners` rather than being derived from an
    // empty Vec: "port not busy" and "busy but the lookup found nobody" are both empty Vecs and
    // must not be conflated (a failed/timed-out lookup is still `busy: true`).
    let mut probed: Vec<(String, u16, bool, Vec<process::PortListener>)> =
        Vec::with_capacity(snapshot.len());
    for (project_id, port) in snapshot {
        let busy = process::port_accepts(port).await;
        let listeners = if busy { process::port_listeners(port, &env).await } else { Vec::new() };
        probed.push((project_id, port, busy, listeners));
    }

    // Pass 2: ONE batched `ps` for every solo-listener pid found across every project this call
    // (plan 041 step 3: "one child for all rows, not one per row").
    let solo_pids: Vec<u32> = probed
        .iter()
        .filter_map(|(_, _, _, listeners)| (listeners.len() == 1).then(|| listeners[0].pid))
        .collect();
    let ps_rows = process::ps_enrich(&solo_pids, &env).await;
    let current_user = current_username();

    Ok(probed
        .into_iter()
        .map(|(project_id, port, busy, listeners)| {
            build_port_status(
                project_id,
                port,
                busy,
                listeners,
                &ps_rows,
                current_user.as_deref(),
                &checked_at,
            )
        })
        .collect())
}

/// The identity used for `PortHolder.sameUser`: std has no portable "who am I" call, so this
/// reads the same env var a login shell would set — `USER` on Unix, `USERNAME` on Windows. `None`
/// (never a guess) when neither is set.
fn current_username() -> Option<String> {
    std::env::var("USER").ok().or_else(|| std::env::var("USERNAME").ok())
}

/// One project's row, assembled from `get_port_status`'s two passes. Never widened into an `Err`:
/// an owner that could not be identified already arrives here as `listener_count == 0`, the
/// documented "owner unknown" state, not a failure.
fn build_port_status(
    project_id: String,
    port: u16,
    busy: bool,
    listeners: Vec<process::PortListener>,
    ps_rows: &HashMap<u32, process::PsInfo>,
    current_user: Option<&str>,
    checked_at: &str,
) -> PortStatus {
    let listener_count = listeners.len();
    // SPEC.md §7: "holder … only when listenerCount === 1 and the lookup parsed" — >1 names
    // nobody (the panel says so instead of guessing), and 0 already IS "owner unknown".
    let holder = (listener_count == 1).then(|| {
        let listener = &listeners[0];
        let ps = ps_rows.get(&listener.pid);
        PortHolder {
            name: listener.name.clone(),
            pid: listener.pid,
            command: ps.map(|r| r.command.clone()),
            started_at: ps.map(|r| r.lstart.clone()),
            parent_exited: ps.map(|r| r.ppid == 1),
            same_user: listener.user.as_deref().zip(current_user).map(|(u, cur)| u == cur),
        }
    });

    PortStatus {
        project_id,
        port,
        busy,
        listener_count,
        holder,
        checked_at: checked_at.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::ProjectRuntime;

    fn sample_project(path: &str) -> Project {
        Project {
            id: "abc123".into(),
            name: "IELTS Coach".into(),
            path: path.into(),
            command: "npm run dev".into(),
            port: 3000,
            url: None,
            update_on_run: true,
            ready_timeout_sec: 60,
            last_lockfile_hash: None,
            last_run_at: None,
            notes: None,
            stack: None,
            folder_id: None,
            folder_name: None,
        }
    }

    #[test]
    fn a_project_absent_from_the_runtime_map_reads_stopped() {
        let runtime = RuntimeMap::new();
        // Deliberately not created — `to_view` must report `pathExists: false` without touching
        // the filesystem beyond a plain existence check.
        let missing_path = std::env::temp_dir().join("hangar-commands-test-does-not-exist");
        let project = sample_project(missing_path.to_str().unwrap());

        let view = to_view(&project, &runtime);

        assert_eq!(view.status, Status::Stopped);
        assert!(!view.path_exists);
    }

    #[test]
    fn a_project_present_in_the_runtime_map_reflects_its_status() {
        let project = sample_project("/tmp/ielts");
        let mut runtime = RuntimeMap::new();
        runtime.insert(
            project.id.clone(),
            ProjectRuntime {
                status: Status::Running,
                ..ProjectRuntime::default()
            },
        );

        let view = to_view(&project, &runtime);

        assert_eq!(view.status, Status::Running);
    }

    // -----------------------------------------------------------------------------------------
    // Plan 020 revision — `guard_update`: a notes-only `update_project` call bypasses the
    // running-project guard (SPEC.md §6 vs. §5); any other field change still goes through it.
    // -----------------------------------------------------------------------------------------

    #[test]
    fn a_notes_only_change_is_permitted_while_running() {
        let stored = sample_project("/tmp/ielts");
        let incoming = Project {
            notes: Some("tried the staging flag".into()),
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        assert!(guard_update(is_run_inert, &stored, Status::Running).is_ok());
    }

    #[test]
    fn a_change_to_any_other_field_is_still_rejected_while_running() {
        let stored = sample_project("/tmp/ielts");
        let incoming = Project {
            port: 3001,
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        let err = guard_update(is_run_inert, &stored, Status::Running)
            .expect_err("a port change must still be guarded while running");
        assert!(err.contains("stop it first"), "got {err:?}");
    }

    #[test]
    fn both_kinds_of_change_are_permitted_while_stopped() {
        let stored = sample_project("/tmp/ielts");
        let notes_only = Project {
            notes: Some("tried the staging flag".into()),
            ..stored.clone()
        };
        let port_changed = Project {
            port: 3001,
            ..stored.clone()
        };
        assert!(
            guard_update(is_run_inert_change(&stored, &notes_only), &stored, Status::Stopped)
                .is_ok()
        );
        assert!(
            guard_update(is_run_inert_change(&stored, &port_changed), &stored, Status::Stopped)
                .is_ok()
        );
    }

    #[test]
    fn clearing_notes_back_to_none_still_counts_as_notes_only() {
        // The Notes panel sends `undefined` (omitted from the wire JSON) for an emptied
        // textarea, which deserializes as `None` — `Some(..)` -> `None` must be notes-only too,
        // not just the `None` -> `Some(..)` direction.
        let stored = Project {
            notes: Some("old note".into()),
            ..sample_project("/tmp/ielts")
        };
        let incoming = Project {
            notes: None,
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        assert!(guard_update(is_run_inert, &stored, Status::Running).is_ok());
    }

    #[test]
    fn an_identical_project_is_a_no_op_permitted_while_running() {
        let stored = sample_project("/tmp/ielts");
        let incoming = stored.clone();
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        assert!(guard_update(is_run_inert, &stored, Status::Running).is_ok());
    }

    // -----------------------------------------------------------------------------------------
    // Plan 028 — folders: `folderId`/`folderName` join `notes` in the run-inert set, and a
    // run-inert `update_project` call now merges rather than replaces (SPEC.md §6, 2026-08-10).
    // -----------------------------------------------------------------------------------------

    #[test]
    fn a_folder_id_only_change_is_permitted_while_running() {
        let stored = sample_project("/tmp/ielts");
        let incoming = Project {
            folder_id: Some("fld_1".into()),
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        assert!(guard_update(is_run_inert, &stored, Status::Running).is_ok());
    }

    #[test]
    fn a_folder_name_only_change_is_permitted_while_running() {
        let stored = sample_project("/tmp/ielts");
        let incoming = Project {
            folder_name: Some("Client Work".into()),
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        assert!(guard_update(is_run_inert, &stored, Status::Running).is_ok());
    }

    #[test]
    fn notes_and_folder_id_together_are_still_run_inert() {
        let stored = sample_project("/tmp/ielts");
        let incoming = Project {
            notes: Some("tried the staging flag".into()),
            folder_id: Some("fld_1".into()),
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        assert!(guard_update(is_run_inert, &stored, Status::Running).is_ok());
    }

    #[test]
    fn a_port_only_change_is_still_rejected_while_running() {
        let stored = sample_project("/tmp/ielts");
        let incoming = Project {
            port: 3001,
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        let err = guard_update(is_run_inert, &stored, Status::Running)
            .expect_err("a port change must still be guarded while running");
        assert!(err.contains("stop it first"), "got {err:?}");
    }

    #[test]
    fn folder_id_cannot_smuggle_a_port_change_past_the_guard() {
        // Proves the run-inert exemption cannot be used as a smuggling route: pairing a
        // run-inert field with a guarded one must still be guarded.
        let stored = sample_project("/tmp/ielts");
        let incoming = Project {
            folder_id: Some("fld_1".into()),
            port: 3001,
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        let err = guard_update(is_run_inert, &stored, Status::Running)
            .expect_err("a folderId + port change must still be guarded while running");
        assert!(err.contains("stop it first"), "got {err:?}");
    }

    #[test]
    fn merging_run_inert_fields_leaves_last_run_at_as_stored() {
        // The bug this guards against: `run.rs` persists `last_run_at` on every Run, but the
        // frontend's copy goes stale for the whole updating->installing->starting window — a
        // run-inert save from that stale copy must not roll it back (SPEC.md §6, plan 028).
        let mut stored = Project {
            last_run_at: Some("2026-08-05T10:00:00Z".into()),
            ..sample_project("/tmp/ielts")
        };
        let incoming = Project {
            last_run_at: None, // stale frontend copy
            folder_id: Some("fld_1".into()),
            ..stored.clone()
        };
        merge_run_inert_fields(&mut stored, incoming);
        assert_eq!(stored.last_run_at.as_deref(), Some("2026-08-05T10:00:00Z"));
        assert_eq!(stored.folder_id.as_deref(), Some("fld_1"));
    }

    // -----------------------------------------------------------------------------------------
    // Plan 032 — the run-inert exemption was unreachable during the very window it exists for:
    // the app-owned fields (`lastRunAt`, `lastLockfileHash`) and `stack.detectedAt` go stale in
    // the frontend's payload for the whole updating/installing/starting window, so a genuinely
    // run-inert save (a note, a folder move) was misclassified as guarded (SPEC.md §6, 2026-08-10).
    // -----------------------------------------------------------------------------------------

    #[test]
    fn a_stale_last_run_at_does_not_defeat_a_folder_move_while_starting() {
        // The headline bug: stored has today's lastRunAt (a Run just stamped it); the caller's
        // payload still carries yesterday's, because status-changed never sends it. Without step
        // 1's normalisation this fails: is_run_inert_change would see lastRunAt differ and
        // guard_update would reject with "stop it first".
        let stored = Project {
            last_run_at: Some("2026-08-10T09:00:00Z".into()),
            ..sample_project("/tmp/ielts")
        };
        let incoming = Project {
            folder_id: Some("fld_1".into()),
            last_run_at: Some("2026-08-05T09:00:00Z".into()), // stale frontend copy
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        assert!(is_run_inert, "a folder move paired with a stale lastRunAt must be run-inert");
        assert!(guard_update(is_run_inert, &stored, Status::Starting).is_ok());
    }

    #[test]
    fn a_stale_last_lockfile_hash_does_not_defeat_a_notes_save_while_installing() {
        let stored = Project {
            last_lockfile_hash: Some("freshhash".into()),
            ..sample_project("/tmp/ielts")
        };
        let incoming = Project {
            notes: Some("tried the staging flag".into()),
            last_lockfile_hash: Some("stalehash".into()), // stale frontend copy
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        assert!(is_run_inert, "a notes save paired with a stale lastLockfileHash must be run-inert");
        assert!(guard_update(is_run_inert, &stored, Status::Installing).is_ok());
    }

    fn sample_stack(detected_at: &str) -> registry::ProjectStack {
        registry::ProjectStack {
            framework: Some("Next".into()),
            libraries: vec!["React".into(), "Tailwind".into()],
            detected_at: detected_at.into(),
        }
    }

    #[test]
    fn a_stack_differing_only_in_detected_at_does_not_defeat_a_folder_move() {
        // registry.rs:559 re-stamps `detected_at` on every Run, same staleness as `lastRunAt`.
        let stored = Project {
            stack: Some(sample_stack("2026-08-10T09:00:00Z")),
            ..sample_project("/tmp/ielts")
        };
        let incoming = Project {
            folder_id: Some("fld_1".into()),
            stack: Some(sample_stack("2026-08-05T09:00:00Z")), // stale frontend copy
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        assert!(is_run_inert, "a stack differing only in detectedAt must be run-inert");
        assert!(guard_update(is_run_inert, &stored, Status::Starting).is_ok());
    }

    #[test]
    fn a_stack_differing_in_framework_is_still_guarded_while_running() {
        let stored = Project {
            stack: Some(sample_stack("2026-08-10T09:00:00Z")),
            ..sample_project("/tmp/ielts")
        };
        let mut changed_stack = sample_stack("2026-08-10T09:00:00Z");
        changed_stack.framework = Some("Remix".into());
        let incoming = Project {
            stack: Some(changed_stack),
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        let err = guard_update(is_run_inert, &stored, Status::Running)
            .expect_err("a genuine framework change must still be guarded while running");
        assert!(err.contains("stop it first"), "got {err:?}");
    }

    #[test]
    fn a_stale_last_run_at_cannot_smuggle_a_port_change_past_the_guard() {
        // The normalisation must not become a smuggling route: pairing a stale app-owned field
        // with a genuinely guarded change must still be guarded.
        let stored = Project {
            last_run_at: Some("2026-08-10T09:00:00Z".into()),
            ..sample_project("/tmp/ielts")
        };
        let incoming = Project {
            port: 3001,
            last_run_at: Some("2026-08-05T09:00:00Z".into()), // stale frontend copy
            ..stored.clone()
        };
        let is_run_inert = is_run_inert_change(&stored, &incoming);
        let err = guard_update(is_run_inert, &stored, Status::Starting)
            .expect_err("a port change must still be guarded even paired with a stale lastRunAt");
        assert!(err.contains("stop it first"), "got {err:?}");
    }

    #[test]
    fn the_replace_branch_preserves_app_owned_fields_from_stored() {
        // A guarded (non-run-inert) edit — e.g. renaming the project — still carries the
        // caller's stale copy of both app-owned fields. Before step 3 the replace branch wrote
        // the payload's `None`s straight over the stored values, mislabelling the card's "last
        // run" time and forcing a spurious reinstall on the next Run.
        let stored = Project {
            last_run_at: Some("2026-08-10T09:00:00Z".into()),
            last_lockfile_hash: Some("freshhash".into()),
            ..sample_project("/tmp/ielts")
        };
        let incoming = Project {
            name: "IELTS Coach (renamed)".into(),
            last_run_at: None,
            last_lockfile_hash: None,
            ..stored.clone()
        };
        let replaced = replace_preserving_app_owned_fields(&stored, incoming);
        assert_eq!(replaced.name, "IELTS Coach (renamed)");
        assert_eq!(replaced.last_run_at.as_deref(), Some("2026-08-10T09:00:00Z"));
        assert_eq!(replaced.last_lockfile_hash.as_deref(), Some("freshhash"));
    }
}
