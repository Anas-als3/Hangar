//! All `#[tauri::command]` functions (SPEC.md §7 — the API is FROZEN).
//!
//! M2 adds the spawn/log slice: `run_project`, `get_log_buffer`, `clear_log_buffer`.
//! The rest (`add_project`, `stop_project`, editor/browser …) arrive with plans 003–005 and must
//! keep the exact names and shapes §7 gives them. `stop_project` is plan 003: there is deliberately
//! no stub that "sort of" stops.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use tauri::{AppHandle, State};
use tokio::sync::Mutex;

use crate::env_resolve::DevEnvCell;
use crate::process::{LogLine, RuntimeMap};
use crate::registry::{self, Project, ProjectView, RegistryError, Settings, Status};

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
    };

    projects.push(project.clone());
    registry::save_projects(&state.config_dir, &projects)?;

    // A brand-new project has never run, so `stopped` (the runtime map's default for an absent
    // entry) is the truthful status — no need to touch the runtime lock to prove it.
    let runtime = state.runtime.lock().await;
    Ok(to_view(&project, &runtime))
}

/// SPEC.md §6 mutation guard vs. §5 notes (plan 020 revision): `guard_mutation` exists because
/// mutating a *running* project can break the run itself — a changed `port` breaks Stop's port
/// verification, a changed `path`/`command` breaks the kill path. §5 now defines `notes` as "a
/// free-text scratchpad, user-owned; never parsed or acted on", so a change that touches only
/// `notes` provably cannot affect a running project — it is exempt from the guard.
///
/// Deliberately not a hand-enumerated field list: normalising `notes` out of both sides and
/// comparing the rest with the derived `PartialEq` means any other field — including ones a
/// future plan adds — is covered by the guard automatically, with nothing to remember to update
/// here.
fn is_notes_only_change(stored: &Project, incoming: &Project) -> bool {
    let mut stored = stored.clone();
    let mut incoming = incoming.clone();
    stored.notes = None;
    incoming.notes = None;
    stored == incoming
}

/// SPEC.md §7 `update_project` / §6 / §10 step 7: "Remove/Edit while status ∉ {stopped, crashed}
/// first shows a confirm ... confirming runs the full §8 kill and waits for verification before
/// removing/saving." The frontend's confirm dialog already called `stop_project` and awaited its
/// verified death before calling this — `guard_mutation` here is what makes that real: a frontend
/// that skipped the confirm (or raced it) cannot save over a project whose tree is still alive.
///
/// Exception: a notes-only change (see `is_notes_only_change`) skips the guard, so the Notes
/// slide-over can autosave while a project is running — the whole point of per-project notes is
/// to record something while or right after testing it (SPEC.md §11).
///
/// Pulled out of `update_project` as plain data in, `Result` out — no `State`/`AppHandle` — so
/// the decision itself is unit-testable without standing up a Tauri app in the test harness.
fn guard_update(stored: &Project, incoming: &Project, status: Status) -> Result<(), String> {
    if is_notes_only_change(stored, incoming) {
        return Ok(());
    }
    crate::run::guard_mutation(status, &stored.name)
}

#[tauri::command]
pub async fn update_project(
    project: Project,
    state: State<'_, AppState>,
) -> Result<ProjectView, String> {
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
    guard_update(&projects[index], &project, status)?;

    if let Some(owner) = registry::port_conflict(&projects, project.port, Some(&project.id)) {
        return Err(format!(
            "Port {} is already used by {}.",
            project.port, owner.name
        ));
    }

    projects[index] = project.clone();
    registry::save_projects(&state.config_dir, &projects)?;

    Ok(to_view(&project, &runtime))
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
        assert!(guard_update(&stored, &incoming, Status::Running).is_ok());
    }

    #[test]
    fn a_change_to_any_other_field_is_still_rejected_while_running() {
        let stored = sample_project("/tmp/ielts");
        let incoming = Project {
            port: 3001,
            ..stored.clone()
        };
        let err = guard_update(&stored, &incoming, Status::Running)
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
        assert!(guard_update(&stored, &notes_only, Status::Stopped).is_ok());
        assert!(guard_update(&stored, &port_changed, Status::Stopped).is_ok());
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
        assert!(guard_update(&stored, &incoming, Status::Running).is_ok());
    }

    #[test]
    fn an_identical_project_is_a_no_op_permitted_while_running() {
        let stored = sample_project("/tmp/ielts");
        let incoming = stored.clone();
        assert!(guard_update(&stored, &incoming, Status::Running).is_ok());
    }
}
