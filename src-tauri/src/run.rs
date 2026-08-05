//! SPEC.md §9 — the exact run sequence (port pre-check, pull, install, spawn, dual-stack ready
//! polling, browser hand-off) — plus the §6 status state machine, which lives here and nowhere else.
//!
//! Plan 002 (M2) implemented the spawn-only slice. Plan 003 (M3) adds the §6 transition table, the
//! Stop sequence (§8 kill + death-then-port verification) and the quit-time stop-everything path.
//! Deliberately NOT here (each named by its owning plan):
//! - the port pre-check, dual-stack ready polling and the browser hand-off — plan 004,
//! - `git pull`, lockfile hashing and installs — plan 006.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};

use crate::commands::AppState;
use crate::process::{
    self, KillTarget, ProjectRuntime, ShellKind, SpawnOutcome, SpawnSpec, StopClaim,
};
use crate::registry::{self, Status};

// ---------------------------------------------------------------------------------------------
// SPEC.md §6 — the status state machine. The single source of truth for what is legal.
// ---------------------------------------------------------------------------------------------

/// Everything that can move a project between statuses. There is no other vocabulary: a command
/// that cannot name its trigger cannot change a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Run clicked.
    Run,
    /// Stop clicked — valid in every active phase, not just `running`.
    Stop,
    /// The port answered and the grace elapsed (§9 step 6). Plan 004 owns the polling that produces
    /// it; M3 fires it straight after a successful spawn.
    Ready,
    /// The registered child exited. `user_stop` is the per-project flag Stop sets *before* killing.
    ChildExit { user_stop: bool },
    /// The run sequence itself failed before/without a child exit (spawn failure now; plan 006's
    /// nonzero install later). Same §6 outcome as an unexpected child exit.
    Failed,
    /// SPEC.md §6 rows "`stopping` + tree death confirmed → `stopped`" and "child exits with the
    /// user-stop flag set → `stopped`". They are one transition — a *verified* stop — and Hangar
    /// announces it from exactly one place: the Stop sequence, after §8 verification has run.
    StopConfirmed,
    /// §8 verification failed (processes alive, or the port still answering).
    KillVerificationFailed,
}

/// A refused transition. Carries the status it was refused from so the caller can name the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub from: Status,
    pub reason: &'static str,
}

impl Rejection {
    pub fn for_project(&self, name: &str) -> String {
        format!("{name} is {} — {}.", status_label(self.from), self.reason)
    }
}

/// The result of an applied transition. `from` matters: §6 distinguishes a Stop from `running`
/// (a normal stop) from a Stop mid-phase (which also logs "Run cancelled by user").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub from: Status,
    pub to: Status,
}

/// SPEC.md §6, as a pure function. Every row of the table is here and nothing else decides a status.
pub fn next_status(from: Status, trigger: Trigger) -> Result<Status, Rejection> {
    use Status::*;

    let refuse = |reason| Err(Rejection { from, reason });

    match trigger {
        // | `stopped`, `crashed` | Run clicked | `updating` → `installing` → `starting` per §9 |
        // M3 has no pull/install phases yet, so the first phase it can honestly enter is `starting`;
        // plan 006 moves the entry point to `updating`. The guard — Run legal from nowhere else — is
        // the part §6 freezes.
        Trigger::Run => match from {
            Stopped | Crashed => Ok(Starting),
            _ => refuse("Run is only valid from stopped or crashed"),
        },

        // | `updating`, `installing`, `starting`, `running` | Stop clicked | `stopping` |
        // | `stop-failed` | Stop clicked | `stopping` (retry the kill) |
        Trigger::Stop => match from {
            Updating | Installing | Starting | Running | StopFailed => Ok(Stopping),
            Stopping => refuse("a stop is already in progress"),
            Stopped | Crashed => refuse("it is not running"),
        },

        // | `starting` | port answers + grace | `running` |
        Trigger::Ready => match from {
            Starting => Ok(Running),
            _ => refuse("a project can only become running from starting"),
        },

        // | `updating`/`installing`/`starting`/`running` | child exits, user-stop flag NOT set |
        // | `crashed` — immediately |
        Trigger::ChildExit { user_stop: false } | Trigger::Failed => match from {
            Updating | Installing | Starting | Running => Ok(Crashed),
            // A crash and a Stop that raced: the Stop sequence owns the outcome and will report
            // `stopped` or `stop-failed` once §8 verification has run.
            Stopping => Ok(Stopping),
            // Nothing left to crash — the status is already settled.
            Stopped | Crashed | StopFailed => Ok(from),
        },

        // | any | child exits, user-stop flag set (incl. quit-time kill) | `stopped` |
        // Held, not announced: §8 requires death to be *verified* before anything says `stopped`,
        // and the Stop sequence that set the flag does exactly that a moment later. What §6
        // guarantees here — that a user Stop can never display as `crashed` — is structural: with
        // the flag set there is no path to `crashed` at all.
        Trigger::ChildExit { user_stop: true } => Ok(from),

        // | `stopping` | tree death confirmed | `stopped` |
        Trigger::StopConfirmed => match from {
            Stopping => Ok(Stopped),
            _ => refuse("only a stop in progress can be confirmed"),
        },

        // | `stopping` | kill verification fails (§8) | `stop-failed` |
        Trigger::KillVerificationFailed => match from {
            Stopping => Ok(StopFailed),
            _ => refuse("only a stop in progress can fail verification"),
        },
    }
}

/// Applies a §6 transition and emits `status-changed`.
///
/// The check and the write happen under ONE lock, which is what makes a double-clicked Run
/// impossible to double-spawn (§6: "the backend enforces it"). `with` runs under the same lock, for
/// the bookkeeping that must be atomic with the transition — clearing the log buffer as a Run
/// begins, setting the user-stop flag as a Stop begins.
pub async fn apply_with<F>(
    app: &AppHandle,
    project_id: &str,
    trigger: Trigger,
    message: Option<String>,
    with: F,
) -> Result<Transition, Rejection>
where
    F: FnOnce(&mut ProjectRuntime),
{
    let transition = {
        let state = app.state::<AppState>();
        let mut runtime = state.runtime.lock().await;
        let entry = runtime.entry(project_id.to_string()).or_default();
        let from = entry.status;
        let to = next_status(from, trigger)?;
        entry.status = to;
        with(entry);
        Transition { from, to }
    };

    // §7: emitted on every transition — and only on a real one.
    if transition.from != transition.to {
        process::emit_status(app, project_id, transition.to, message);
    }
    Ok(transition)
}

pub async fn apply(
    app: &AppHandle,
    project_id: &str,
    trigger: Trigger,
    message: Option<String>,
) -> Result<Transition, Rejection> {
    apply_with(app, project_id, trigger, message, |_| {}).await
}

/// SPEC.md §6: "Remove/Edit while status ∉ {`stopped`, `crashed`} first shows a confirm … confirming
/// runs the full §8 kill and **waits for verification** before removing/saving."
///
/// This is the backend half of that rule, and the reason it can be trusted: plan 005's
/// `remove_project` / `update_project` call it *before* touching the registry, so even a frontend
/// that skipped its confirm dialog cannot mutate a project whose tree is still alive. The confirm
/// flow itself calls `stop_project` first, which only returns Ok once death is verified.
// Unused until plan 005 adds `remove_project` / `update_project`; wired and tested now so the rule
// exists before the commands that must obey it.
#[allow(dead_code)]
pub fn guard_mutation(status: Status, name: &str) -> Result<(), String> {
    if matches!(status, Status::Stopped | Status::Crashed) {
        Ok(())
    } else {
        Err(format!(
            "{name} is {} — stop it first.",
            status_label(status)
        ))
    }
}

/// SPEC.md §9 steps 0 and 4, as far as M3 goes.
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

    // ---- §6 guard + claim, atomically ---------------------------------------------------------
    //
    // The claim moves the card to `starting`, which is precisely what makes Stop legal (§6). Every
    // await below therefore runs inside a window where the user can press Stop but there is no
    // child to kill yet — the `lastRunAt` write, the login-shell environment resolution (a cold
    // cell shells out with a 5 s budget) and, once plans 004/006 land, the whole pull/install
    // phase. `spawn_in_flight` is the run sequence's claim on that window: a Stop arriving inside
    // it parks on this receiver rather than verifying a death that has not happened, and the run
    // sequence cancels itself the moment it notices (SPEC.md §12, "Stop clicked during
    // updating/installing/starting").
    //
    // The sender is held for the whole body of this function: every return path drops it, which
    // releases a parked Stop even on a path that forgot to.
    let (spawn_claim, spawn_in_flight) = tokio::sync::watch::channel(false);
    apply_with(app, &project.id, Trigger::Run, None, |entry| {
        entry.begin_run(spawn_in_flight)
    })
    .await
    .map_err(|rejection| rejection.for_project(&project.name))?;

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
    //
    // Last look before anything is created: a Stop that landed while the environment was resolving
    // costs zero processes if it is noticed here. `had_child: false` is the honest input to §8
    // verification — there demonstrably never was a child for this run.
    if stop_is_pending(app, &project.id).await {
        return cancel_run(app, &project, KillTarget::default()).await;
    }

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
            // Nothing was created, so the pre-registration window closes with no kill target — and
            // `had_child` stays false, which is the honest answer for any Stop still parked on it.
            let _ = apply_with(app, &project.id, Trigger::Failed, Some(e.clone()), |entry| {
                entry.spawn_in_flight = None;
            })
            .await;
            return Err(e);
        }
    };

    let mut child = spawned.child;
    let pid = child.id();
    // The exit watcher signals this once it has reaped the child; the §8 kill sequence awaits it
    // instead of calling `wait()` a second time on a Child it does not own.
    let (exit_tx, exit_rx) = tokio::sync::watch::channel(false);
    // SPEC.md §8/§12: `/bin/sh` always exists, so the spawn above succeeds even when `npm` does
    // not — the shell reports "command not found" and exits 127. The exit watcher prints this
    // PATH in that case; without it, the single most important error message in the app is
    // missing. See `process::is_tool_not_found_exit`.
    let outcome = {
        let mut runtime = state.runtime.lock().await;
        let entry = runtime.entry(project.id.clone()).or_default();
        #[cfg(windows)]
        {
            entry.job = spawned.job;
        }
        // Publishing the kill target and reading the user-stop flag happen under ONE lock. Split
        // them and a Stop can slip between, find nothing registered, and announce a stop while this
        // very tree comes up behind it.
        entry.register_child(pid, exit_rx, path_searched.clone())
    };

    let pipeline = process::attach_log_pipeline(app, &project.id, &mut child);

    if outcome == SpawnOutcome::CancelRun {
        // A Stop was claimed while this run was working: the card already says `stopping` and the
        // Stop sequence is parked waiting for us, because at the moment it looked there was nothing
        // to signal. This tree is reachable from nowhere else. The exit watcher starts first — it
        // owns `wait()`, so it is both the reaper and what the kill below awaits (§8).
        process::spawn_exit_watcher(app.clone(), project.id.clone(), child, pipeline, exit_tx);
        let target = {
            let mut runtime = state.runtime.lock().await;
            runtime
                .entry(project.id.clone())
                .or_default()
                .take_kill_target()
        };
        return cancel_run(app, &project, target).await;
    }

    // PLAN 004 OWNS THIS LINE. Ready-detection — dual-stack port polling racing the child's exit,
    // the attempt-counted timeout, the 300 ms grace and opening the browser — is plan 004's scope.
    // Until then a successful spawn is the most this milestone can honestly claim, so the card goes
    // straight to `running`. Do not add a placeholder poll here.
    //
    // The rejection is ignored on purpose: a Stop clicked in the moments between the registration
    // above and this line leaves the project in `stopping`, where `Ready` is illegal — and the Stop
    // must win. It can do so safely now: that Stop found a registered kill target.
    let _ = apply(app, &project.id, Trigger::Ready, None).await;

    // Started last on purpose: an instantly-exiting command must not have its `crashed` transition
    // overwritten by the `running` line above.
    process::spawn_exit_watcher(app.clone(), project.id.clone(), child, pipeline, exit_tx);

    // Held until here so that every early return above wakes a parked Stop by dropping it.
    drop(spawn_claim);
    Ok(())
}

/// Has a Stop already been claimed for the run currently in flight? (SPEC.md §6 makes Stop legal
/// from the moment the `Run` claim lands, which is several awaits before there is a child.)
async fn stop_is_pending(app: &AppHandle, project_id: &str) -> bool {
    let state = app.state::<AppState>();
    let runtime = state.runtime.lock().await;
    runtime
        .get(project_id)
        .is_some_and(ProjectRuntime::run_cancelled)
}

/// The run sequence cancelling itself because a Stop landed while it was working.
///
/// It runs the *same* §8 sequence a Stop does — kill, reap, confirmed death, then the port — and
/// announces through the same `StopConfirmed` / `KillVerificationFailed` triggers, so a cancelled
/// run can never report `stopped` on a tree it has not verified.
async fn cancel_run(
    app: &AppHandle,
    project: &registry::Project,
    target: KillTarget,
) -> Result<(), String> {
    finish_stop(
        app,
        &project.id,
        &project.name,
        Some(project.port),
        true,
        target,
    )
    .await
}

// ---------------------------------------------------------------------------------------------
// SPEC.md §8 — the Stop sequence: kill the tree, verify death, then the port, then the status
// ---------------------------------------------------------------------------------------------

/// Stop is valid in every active phase (§6), and from `stop-failed` as a retry.
///
/// Order is not negotiable (SPEC.md §8):
/// 1. set the user-stop flag and `stopping` — *before* the kill, so the exit watcher can never see
///    the child die with the flag still false and label a deliberate Stop as `crashed`;
/// 2. kill the tree (Job Object / process group) and await the reap;
/// 3. confirm **process death**;
/// 4. only then confirm the **port**, because leaked children that never listen (esbuild service,
///    file watchers) would sail through a port-only check;
/// 5. `stopped` if both hold, `stop-failed` otherwise — never a silent "stopped".
pub async fn stop_project(app: &AppHandle, project_id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();

    let project = {
        let projects = state.projects.lock().await;
        projects.iter().find(|p| p.id == project_id).cloned()
    };
    let name = project
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| project_id.to_string());
    // A project removed from the registry mid-stop has no port left to verify; the death check
    // still applies and is the half that actually matters.
    let port = project.as_ref().map(|p| p.port);

    // ---- 1. claim the stop -------------------------------------------------------------------
    let mut claim = None;
    let transition = apply_with(app, project_id, Trigger::Stop, None, |entry| {
        claim = Some(entry.claim_stop());
    })
    .await
    .map_err(|rejection| rejection.for_project(&name))?;

    let mut cancelled_mid_phase = matches!(
        transition.from,
        Status::Updating | Status::Installing | Status::Starting
    );

    let target = match claim.expect("the Stop claim runs under the transition's lock") {
        StopClaim::Kill(target) => target,
        StopClaim::AwaitSpawn(spawn_in_flight) => {
            // The run sequence is between its `starting` claim and its child's registration, so
            // there is nothing to signal *yet* — and reporting that as a verified death is how a
            // live tree ends up behind a `stopped` card, orphaned for the app's lifetime. The flag
            // set above is already visible to the spawn side; wait for it to notice and settle.
            process::append_system(
                app,
                project_id,
                "waiting for the run that is still starting to cancel",
            )
            .await;
            if !await_spawn_cancel(spawn_in_flight).await {
                let message = format!(
                    "Couldn't confirm {name} stopped: the run that was starting did not respond \
                     within {} s. Press Stop again to retry.",
                    SPAWN_CANCEL_BUDGET.as_secs()
                );
                process::append_system(app, project_id, message.clone()).await;
                let _ = apply(app, project_id, Trigger::KillVerificationFailed, None).await;
                return Err(message);
            }

            // The run sequence settles the outcome itself, through the same §8 verification and the
            // same §6 triggers (see `cancel_run`). If it did, honour its verdict.
            let settled = {
                let runtime = state.runtime.lock().await;
                runtime.get(project_id).map(|entry| entry.status)
            };
            match settled {
                Some(Status::Stopped) => return Ok(()),
                Some(Status::StopFailed) => {
                    return Err(format!(
                        "Couldn't confirm {name} stopped — see the log. Press Stop again to retry."
                    ))
                }
                _ => {}
            }

            // It returned without settling (a spawn failure, say). Whatever it left behind is ours
            // to finish — including the case where it left nothing, which `had_child` reports
            // honestly rather than as a confirmed death.
            cancelled_mid_phase = true;
            let mut runtime = state.runtime.lock().await;
            runtime
                .entry(project_id.to_string())
                .or_default()
                .take_kill_target()
        }
    };

    finish_stop(app, project_id, &name, port, cancelled_mid_phase, target).await
}

/// How long a Stop waits for a run sequence to notice it and cancel. Generous on purpose: the run
/// side's own cancellation runs the full §8 kill (5 s SIGTERM grace + 5 s reap + 3 s death check +
/// the port probe) before it releases the waiter.
const SPAWN_CANCEL_BUDGET: Duration = Duration::from_secs(20);

/// True if the run sequence settled (or simply returned, which drops the sender) inside the budget.
async fn await_spawn_cancel(mut spawn_in_flight: tokio::sync::watch::Receiver<bool>) -> bool {
    tokio::time::timeout(SPAWN_CANCEL_BUDGET, async move {
        // `Err` means every sender is gone, i.e. the run sequence has returned — also a settlement.
        let _ = spawn_in_flight.wait_for(|settled| *settled).await;
    })
    .await
    .is_ok()
}

/// Steps 2–5 of the Stop sequence, shared by `stop_project` and by a run sequence cancelling itself.
/// There is exactly one implementation so a cancelled run cannot verify less than a Stop does.
async fn finish_stop(
    app: &AppHandle,
    project_id: &str,
    name: &str,
    port: Option<u16>,
    cancelled_mid_phase: bool,
    target: KillTarget,
) -> Result<(), String> {
    // ---- 2. kill -----------------------------------------------------------------------------
    let mut outcome = process::kill_tree(target).await;
    for note in std::mem::take(&mut outcome.notes) {
        process::append_system(app, project_id, note).await;
    }

    // ---- 3./4. verify: death FIRST, then the port ---------------------------------------------
    let port_still_answers = if outcome.death_confirmed {
        match port {
            Some(port) => process::port_accepts(port).await,
            None => false,
        }
    } else {
        // Not even asked: with processes still alive the port answer is meaningless.
        true
    };

    // ---- 5. status ---------------------------------------------------------------------------
    if process::stop_is_verified(outcome.death_confirmed, port_still_answers) {
        if cancelled_mid_phase {
            // §6: "log line 'Run cancelled by user' if user-stopped mid-phase".
            process::append_system(app, project_id, "Run cancelled by user").await;
        }
        process::append_system(
            app,
            project_id,
            match port {
                Some(port) => format!("stopped — the process tree is gone and port {port} is free"),
                None => "stopped — the process tree is gone".to_string(),
            },
        )
        .await;

        // Verified — and this is one of only two places the kill primitive may be retired.
        if let Err(rejection) = apply_with(app, project_id, Trigger::StopConfirmed, None, |entry| {
            entry.clear_kill_target()
        })
        .await
        {
            process::append_system(
                app,
                project_id,
                format!(
                    "the status had already moved to {} — leaving it alone",
                    status_label(rejection.from)
                ),
            )
            .await;
        }
        return Ok(());
    }

    let reason = if !outcome.death_confirmed {
        "some processes from this project are still alive".to_string()
    } else {
        match port {
            Some(port) => format!("port {port} is still accepting connections"),
            None => "the port is still in use".to_string(),
        }
    };
    let message = format!("Couldn't confirm {name} stopped: {reason}. Press Stop again to retry.");

    process::append_system(app, project_id, message.clone()).await;
    // §6: "`stop-failed` | Stop clicked | `stopping` — retry the kill". A retry can only kill what
    // it still owns, so the primitive goes back into the map instead of being dropped here.
    let _ = apply_with(
        app,
        project_id,
        Trigger::KillVerificationFailed,
        Some(reason),
        |entry| entry.restore_kill_target(&mut outcome),
    )
    .await;

    Err(message)
}

/// Total budget for the quit-time kill of every project (SPEC.md §8, app-quit path). A wedged
/// verification must not hold the app open.
const QUIT_KILL_BUDGET: Duration = Duration::from_secs(15);

/// Every project Stop is currently legal for, as `(id, name)` — the projects that make quitting a
/// confirm-first affair (SPEC.md §8/§12 "App quit while running").
pub async fn stoppable_projects(app: &AppHandle) -> Vec<(String, String)> {
    let state = app.state::<AppState>();
    // projects → runtime, the same lock order as `get_projects`.
    let projects = state.projects.lock().await;
    let runtime = state.runtime.lock().await;

    runtime
        .iter()
        .filter(|(_, entry)| next_status(entry.status, Trigger::Stop).is_ok())
        .map(|(id, _)| {
            let name = projects
                .iter()
                .find(|p| &p.id == id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| id.clone());
            (id.clone(), name)
        })
        .collect()
}

/// SPEC.md §8 quit path: kill all trees, phase children included. Runs the full Stop sequence for
/// each project concurrently so one slow tree does not serialize the rest, bounded overall so the
/// app can always finish quitting.
pub async fn stop_all(app: &AppHandle) {
    let mut tasks = Vec::new();
    for (id, _) in stoppable_projects(app).await {
        let app = app.clone();
        tasks.push(tauri::async_runtime::spawn(async move {
            let _ = stop_project(&app, &id).await;
        }));
    }

    let _ = tokio::time::timeout(QUIT_KILL_BUDGET, async {
        for task in tasks {
            let _ = task.await;
        }
    })
    .await;
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

    // -----------------------------------------------------------------------------------------
    // SPEC.md §6 — the transition table, row by row
    // -----------------------------------------------------------------------------------------

    const EVERY_STATUS: [Status; 8] = [
        Status::Stopped,
        Status::Updating,
        Status::Installing,
        Status::Starting,
        Status::Running,
        Status::Stopping,
        Status::Crashed,
        Status::StopFailed,
    ];

    /// The four statuses a Run is *not* allowed from.
    const ACTIVE: [Status; 4] = [
        Status::Updating,
        Status::Installing,
        Status::Starting,
        Status::Running,
    ];

    #[test]
    fn run_is_legal_only_from_stopped_or_crashed() {
        assert_eq!(next_status(Status::Stopped, Trigger::Run), Ok(Status::Starting));
        assert_eq!(next_status(Status::Crashed, Trigger::Run), Ok(Status::Starting));

        for from in EVERY_STATUS {
            if matches!(from, Status::Stopped | Status::Crashed) {
                continue;
            }
            assert!(
                next_status(from, Trigger::Run).is_err(),
                "Run must be rejected from {}",
                status_label(from)
            );
        }
    }

    #[test]
    fn run_is_rejected_from_running() {
        // The double-click case, called out explicitly: it must be impossible to double-spawn.
        let rejection = next_status(Status::Running, Trigger::Run).unwrap_err();
        assert_eq!(rejection.from, Status::Running);
        assert_eq!(
            rejection.for_project("IELTS Coach"),
            "IELTS Coach is running — Run is only valid from stopped or crashed."
        );
    }

    #[test]
    fn stop_is_legal_from_every_active_phase_and_from_stop_failed() {
        for from in ACTIVE {
            assert_eq!(
                next_status(from, Trigger::Stop),
                Ok(Status::Stopping),
                "Stop must be valid from {}",
                status_label(from)
            );
        }
        // §6: "`stop-failed` | Stop clicked | `stopping` — retry the kill".
        assert_eq!(
            next_status(Status::StopFailed, Trigger::Stop),
            Ok(Status::Stopping)
        );
    }

    #[test]
    fn stop_is_rejected_when_there_is_nothing_to_stop_or_a_stop_is_running() {
        for from in [Status::Stopped, Status::Crashed, Status::Stopping] {
            assert!(next_status(from, Trigger::Stop).is_err());
        }
    }

    #[test]
    fn a_child_exit_without_the_user_stop_flag_crashes_every_active_phase() {
        for from in ACTIVE {
            assert_eq!(
                next_status(from, Trigger::ChildExit { user_stop: false }),
                Ok(Status::Crashed),
                "an unexpected exit from {} is a crash",
                status_label(from)
            );
            // A failure inside the run sequence itself lands the same way.
            assert_eq!(next_status(from, Trigger::Failed), Ok(Status::Crashed));
        }
    }

    #[test]
    fn a_child_exit_with_the_user_stop_flag_never_crashes() {
        // The rule this whole plan exists to protect (§6): "a user Stop must never display as
        // `crashed`". With the flag set there is no path to `crashed` from ANY status — the Stop
        // sequence announces `stopped` itself once §8 verification has confirmed the tree is dead.
        for from in ACTIVE {
            assert_ne!(
                next_status(from, Trigger::ChildExit { user_stop: true }).unwrap(),
                Status::Crashed,
                "a user-stopped exit from {} must not read as a crash",
                status_label(from)
            );
        }
        // The status is *held*, from every state, until verification decides between `stopped`
        // (`Trigger::StopConfirmed`) and `stop-failed`.
        for from in EVERY_STATUS {
            assert_eq!(
                next_status(from, Trigger::ChildExit { user_stop: true }),
                Ok(from),
                "the Stop sequence owns the announcement"
            );
        }
        assert_eq!(
            next_status(Status::Stopping, Trigger::StopConfirmed),
            Ok(Status::Stopped),
            "and what it announces for a verified kill is `stopped`"
        );
    }

    #[test]
    fn a_crash_that_races_a_stop_leaves_the_stop_sequence_in_charge() {
        assert_eq!(
            next_status(Status::Stopping, Trigger::ChildExit { user_stop: false }),
            Ok(Status::Stopping)
        );
    }

    #[test]
    fn an_exit_after_the_status_settled_changes_nothing() {
        for from in [Status::Stopped, Status::Crashed, Status::StopFailed] {
            assert_eq!(
                next_status(from, Trigger::ChildExit { user_stop: false }),
                Ok(from)
            );
        }
    }

    #[test]
    fn verified_death_stops_and_failed_verification_is_stop_failed() {
        // §8: death-confirmed + port-free → `stopped`.
        assert_eq!(
            next_status(Status::Stopping, Trigger::StopConfirmed),
            Ok(Status::Stopped)
        );
        // death-confirmed + port still answering, or death not confirmed → `stop-failed`.
        assert_eq!(
            next_status(Status::Stopping, Trigger::KillVerificationFailed),
            Ok(Status::StopFailed)
        );

        // Neither is reachable from anywhere else — a stale kill task cannot overwrite a status a
        // later Run has already moved on.
        for from in EVERY_STATUS {
            if from == Status::Stopping {
                continue;
            }
            assert!(next_status(from, Trigger::StopConfirmed).is_err());
            assert!(next_status(from, Trigger::KillVerificationFailed).is_err());
        }
    }

    /// Plan 003's test plan, "Kill verification result mapping" — the two §8 checks, end to end,
    /// onto the two statuses §6 allows a `stopping` project to reach.
    #[test]
    fn kill_verification_maps_death_and_port_onto_stopped_or_stop_failed() {
        fn verdict(death_confirmed: bool, port_still_answers: bool) -> Status {
            let trigger = if process::stop_is_verified(death_confirmed, port_still_answers) {
                Trigger::StopConfirmed
            } else {
                Trigger::KillVerificationFailed
            };
            next_status(Status::Stopping, trigger).unwrap()
        }

        assert_eq!(verdict(true, false), Status::Stopped, "death + free port");
        assert_eq!(
            verdict(true, true),
            Status::StopFailed,
            "death confirmed but the port still answers — something else owns it"
        );
        // Death not confirmed is `stop-failed` whatever the port says: a port-only check is the
        // false proxy §8 exists to forbid, since leaked watchers never listen on it at all.
        assert_eq!(verdict(false, true), Status::StopFailed);
        assert_eq!(verdict(false, false), Status::StopFailed);
    }

    #[test]
    fn ready_is_reachable_only_from_starting() {
        assert_eq!(next_status(Status::Starting, Trigger::Ready), Ok(Status::Running));
        for from in EVERY_STATUS {
            if from == Status::Starting {
                continue;
            }
            assert!(next_status(from, Trigger::Ready).is_err());
        }
    }

    #[test]
    fn remove_and_edit_are_guarded_unless_the_project_is_settled() {
        // §6: Remove/Edit while status ∉ {stopped, crashed} must confirm-and-stop first.
        assert!(guard_mutation(Status::Stopped, "IELTS Coach").is_ok());
        assert!(guard_mutation(Status::Crashed, "IELTS Coach").is_ok());
        for from in [
            Status::Updating,
            Status::Installing,
            Status::Starting,
            Status::Running,
            Status::Stopping,
            Status::StopFailed,
        ] {
            let err = guard_mutation(from, "IELTS Coach").unwrap_err();
            assert!(err.starts_with("IELTS Coach is "), "got {err}");
            assert!(err.ends_with(" — stop it first."), "got {err}");
        }
    }
}
