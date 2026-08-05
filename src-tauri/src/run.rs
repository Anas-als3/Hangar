//! SPEC.md §9 — the exact run sequence (port pre-check, pull, install, spawn, dual-stack ready
//! polling, browser hand-off) plus enforcement of the §6 status state machine.
//!
//! Plan 002 (M2) implements the spawn-only slice: the §6 guard, clearing the log buffer, `lastRunAt`,
//! `starting`, and the spawn itself. Deliberately NOT here (each named by its owning plan):
//! - the port pre-check, dual-stack ready polling and the browser hand-off — plan 004,
//! - `git pull`, lockfile hashing and installs — plan 006,
//! - anything that kills a process — plan 003.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};

use crate::commands::AppState;
use crate::process::{self, ShellKind, SpawnSpec};
use crate::registry::{self, Status};

/// SPEC.md §9 steps 0 and 4, as far as M2 goes.
pub async fn run_project(app: &AppHandle, project_id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();

    // ---- §9 step 0: the project must exist and its folder must still be there ------------------
    let project = {
        let projects = state.projects.lock().await;
        projects
            .iter()
            .find(|p| p.id == project_id)
            .cloned()
            .ok_or_else(|| format!("no project with id {project_id}"))?
    };

    if !Path::new(&project.path).exists() {
        return Err(format!(
            "{} can't run: the folder {} no longer exists.",
            project.name, project.path
        ));
    }

    // ---- §6 guard --------------------------------------------------------------------------
    // Claimed while the runtime lock is held: a double-clicked Run must be impossible to
    // double-spawn, so the check and the claim cannot be separated by an await.
    {
        let mut runtime = state.runtime.lock().await;
        let entry = runtime.entry(project.id.clone()).or_default();
        if !matches!(entry.status, Status::Stopped | Status::Crashed) {
            return Err(format!(
                "{} is {} — Run is only valid from stopped or crashed.",
                project.name,
                status_label(entry.status)
            ));
        }
        entry.status = Status::Starting;
        entry.user_stop = false;
        // §8 buffer lifecycle: cleared at the start of each Run.
        entry.logs.clear();
    }

    // The claim above is silent; this is the transition the frontend sees (§7).
    process::set_status(app, &project.id, Status::Starting, None).await;

    // ---- §5/§6: lastRunAt is set when entering `starting` -------------------------------------
    let started_at = iso8601_utc(SystemTime::now());
    let persist_error = {
        let mut projects = state.projects.lock().await;
        if let Some(p) = projects.iter_mut().find(|p| p.id == project.id) {
            p.last_run_at = Some(started_at);
        }
        registry::save_projects(&state.config_dir, &projects).err()
    };
    if let Some(e) = persist_error {
        process::append_system(app, &project.id, format!("could not save lastRunAt: {e}")).await;
    }

    // ---- §8 environment resolution ------------------------------------------------------------
    let (env, path_searched, notes) = {
        let dev_env = state.dev_env.get().await;
        (
            dev_env.vars.clone(),
            dev_env.effective_path(),
            dev_env.notes.clone(),
        )
    };
    for note in notes {
        process::append_system(app, &project.id, note).await;
    }

    // ---- §9 step 4: spawn ---------------------------------------------------------------------
    process::append_system(app, &project.id, format!("$ {}", project.command)).await;

    let spec = SpawnSpec {
        command: project.command.clone(),
        cwd: Some(PathBuf::from(&project.path)),
        env,
        extra_env: Vec::new(),
        // Hangar must be able to tree-kill the dev server (plan 003), so: own process group on
        // Unix, Job Object on Windows.
        long_lived: true,
        kill_on_drop: false,
        shell: ShellKind::Default,
    };

    let spawned = match process::spawn(&spec) {
        Ok(spawned) => spawned,
        Err(e) => {
            // SPEC.md §8: if a tool resolves to nothing, the error line must show the PATH searched.
            process::append_system(app, &project.id, format!("{e}\nPATH searched: {path_searched}"))
                .await;
            process::set_status(app, &project.id, Status::Crashed, Some(e.clone())).await;
            return Err(e);
        }
    };

    let mut child = spawned.child;
    let pid = child.id();
    {
        let mut runtime = state.runtime.lock().await;
        let entry = runtime.entry(project.id.clone()).or_default();
        entry.child_pid = pid;
        #[cfg(windows)]
        {
            entry.job = spawned.job;
        }
    }

    process::attach_log_pipeline(app, &project.id, &mut child);

    // PLAN 004 OWNS THIS LINE. Ready-detection — dual-stack port polling racing the child's exit,
    // the attempt-counted timeout, the 300 ms grace and opening the browser — is plan 004's scope.
    // Until then a successful spawn is the most this milestone can honestly claim, so the card goes
    // straight to `running`. Do not add a placeholder poll here.
    process::set_status(app, &project.id, Status::Running, None).await;

    // Started last on purpose: an instantly-exiting command must not have its `crashed` transition
    // overwritten by the `running` line above.
    process::spawn_exit_watcher(app.clone(), project.id.clone(), child);

    Ok(())
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Stopped => "stopped",
        Status::Updating => "updating",
        Status::Installing => "installing",
        Status::Starting => "starting",
        Status::Running => "running",
        Status::Stopping => "stopping",
        Status::Crashed => "crashed",
        Status::StopFailed => "stop-failed",
    }
}

/// `lastRunAt` is an ISO-8601 UTC string (SPEC.md §5). Formatted by hand rather than pulling in a
/// date crate for one field — the civil-from-days conversion is Howard Hinnant's standard algorithm.
pub fn iso8601_utc(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let days = secs.div_euclid(86_400);
    let time_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60,
    );

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> String {
        iso8601_utc(UNIX_EPOCH + std::time::Duration::from_secs(secs))
    }

    #[test]
    fn formats_the_epoch() {
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn formats_known_timestamps() {
        assert_eq!(at(1_000_000_000), "2001-09-09T01:46:40Z");
        // 2026-08-05T12:34:56Z — a leap-year-adjacent date well past the epoch.
        assert_eq!(at(1_785_933_296), "2026-08-05T12:34:56Z");
        // 29 February on a leap year.
        assert_eq!(at(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn status_labels_match_the_wire_values() {
        assert_eq!(status_label(Status::StopFailed), "stop-failed");
        assert_eq!(
            serde_json::to_string(&Status::StopFailed).unwrap(),
            "\"stop-failed\""
        );
    }
}
