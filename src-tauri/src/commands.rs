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
