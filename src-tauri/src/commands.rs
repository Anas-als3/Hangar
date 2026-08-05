//! All `#[tauri::command]` functions (SPEC.md §7 — the API is FROZEN).
//!
//! M1 implements only the read-only slice: `get_projects`, `get_settings`, `set_settings`.
//! The rest (`add_project`, `run_project`, `stop_project`, log buffer, editor/browser …) arrive
//! with plans 002–005 and must keep the exact names and shapes §7 gives them.

use std::path::{Path, PathBuf};

use tauri::State;
use tokio::sync::Mutex;

use crate::registry::{self, Project, ProjectView, RegistryError, Settings, Status};

/// Managed state (SPEC.md §4). The mutexes are `tokio::sync::Mutex` — never the blocking std one:
/// from plan 002 onwards kill/wait sequences `.await` while this state is consulted, and a
/// blocking guard may never be held across an `.await`.
pub struct AppState {
    pub config_dir: PathBuf,
    pub projects: Mutex<Vec<Project>>,
    pub settings: Mutex<Settings>,
    /// Set once at startup when `projects.json` could not be parsed; drives the §11 banner.
    pub registry_error: Option<RegistryError>,
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
        }
    }
}

/// Derived, never persisted (SPEC.md §5). At M1 there is no process manager yet, so every project
/// is `stopped`; plan 002 replaces this with the real per-project status.
fn to_view(project: &Project) -> ProjectView {
    ProjectView {
        project: project.clone(),
        status: Status::Stopped,
        path_exists: Path::new(&project.path).exists(),
    }
}

#[tauri::command]
pub async fn get_projects(state: State<'_, AppState>) -> Result<Vec<ProjectView>, String> {
    let projects = state.projects.lock().await;
    // Array order is the display order — no sorting, ever (SPEC.md §11).
    Ok(projects.iter().map(to_view).collect())
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
