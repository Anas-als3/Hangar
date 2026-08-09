//! SPEC.md §9 — the exact run sequence (port pre-check, pull, install, spawn, dual-stack ready
//! polling, browser hand-off) — plus the §6 status state machine, which lives here and nowhere else.
//!
//! Plan 002 (M2) implemented the spawn-only slice. Plan 003 (M3) adds the §6 transition table, the
//! Stop sequence (§8 kill + death-then-port verification) and the quit-time stop-everything path.
//! Deliberately NOT here (each named by its owning plan):
//! - the port pre-check, dual-stack ready polling and the browser hand-off — plan 004,
//! - `git pull`, lockfile hashing and installs — plan 006.

use std::future::Future;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::watch;

use crate::commands::AppState;
use crate::env_resolve::EnvMap;
use crate::process::{
    self, KillTarget, LockfileKind, ProjectRuntime, ShellKind, SpawnOutcome, SpawnSpec, StopClaim,
};
use crate::registry::{self, Project, Status};

// ---------------------------------------------------------------------------------------------
// SPEC.md §9 steps 5-7 — ready-detection constants. Requirements, not tuning knobs.
// ---------------------------------------------------------------------------------------------

/// §9 step 5: "Poll both `127.0.0.1:port` and `[::1]:port` every 500 ms".
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// §9 step 6: "Wait 300 ms grace, then: status `running`". Not decoration — a server that has just
/// bound its socket is often still finishing its first compile, and opening the tab into a
/// connection-reset is worse than opening it 300 ms later.
pub const READY_GRACE: Duration = Duration::from_millis(300);

/// §9 step 5 and §12 ("System sleep during `starting`"): the budget counts *completed poll
/// attempts*, and a gap this long between two of them means the machine was suspended — the server
/// was not given that time and must not be charged for it.
pub const SLEEP_GAP: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------------------------
// SPEC.md §6 — the status state machine. The single source of truth for what is legal.
// ---------------------------------------------------------------------------------------------

/// Everything that can move a project between statuses. There is no other vocabulary: a command
/// that cannot name its trigger cannot change a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Run clicked. Carries the phase this run actually starts in — SPEC.md §9 steps 2-3 only
    /// enter `updating`/`installing` when there is real work to do (a pull that will really run, an
    /// install that will really run); §12's "Not a git repo | skip pull silently" and "no lockfile
    /// found | skip installing" both mean the corresponding status is never observed at all, not
    /// entered-then-immediately-left. `run_project` decides the payload before claiming the run,
    /// once for the whole run — see its `first_phase` computation.
    Run(Status),
    /// Stop clicked — valid in every active phase, not just `running`.
    Stop,
    /// The run sequence advancing itself once a phase's own work is done (SPEC.md §9 steps 2-4):
    /// `updating` → `installing`/`starting`, or `installing` → `starting`. Not a user-facing event
    /// — the "cause" is simply "the previous phase finished" — but it still goes through this same
    /// table rather than a parallel status write, so it stays the single source of truth.
    PhaseAdvance(Status),
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
        // The guard — Run legal from nowhere else, and only into one of the three real phases — is
        // the part §6 freezes; *which* of the three is `run_project`'s call, made once before the
        // claim (see `Trigger::Run`'s doc).
        Trigger::Run(first_phase) => match from {
            Stopped | Crashed if matches!(first_phase, Updating | Installing | Starting) => {
                Ok(first_phase)
            }
            Stopped | Crashed => refuse("invalid first phase for Run"),
            _ => refuse("Run is only valid from stopped or crashed"),
        },

        // The run sequence's own §9 steps 2-4 progression — see `Trigger::PhaseAdvance`'s doc.
        Trigger::PhaseAdvance(to) => match (from, to) {
            (Updating, Installing) | (Updating, Starting) | (Installing, Starting) => Ok(to),
            _ => refuse("phase advance is only valid updating->installing/starting or installing->starting"),
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
/// `remove_project` / `update_project` (`commands.rs`) call it *before* touching the registry, so
/// even a frontend that skipped its confirm dialog cannot mutate a project whose tree is still
/// alive. The confirm flow itself calls `stop_project` first, which only returns Ok once death is
/// verified.
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

// ---------------------------------------------------------------------------------------------
// SPEC.md §9 step 5 — the ready budget, counted in attempts rather than wall-clock
// ---------------------------------------------------------------------------------------------

/// The remaining poll attempts for one `starting` phase.
///
/// SPEC.md §9 step 5 is specific: *"The timeout budget is counted in **completed poll attempts**
/// (`readyTimeoutSec × 2` attempts), not wall-clock — a poll gap over 5 s (system slept) does not
/// count against the budget."* A wall-clock deadline expires the instant a laptop wakes, killing a
/// perfectly healthy server that was never given the time it was charged for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptBudget {
    remaining: u32,
}

impl AttemptBudget {
    /// Two attempts per second, because [`POLL_INTERVAL`] is 500 ms.
    pub fn new(ready_timeout_sec: u32) -> Self {
        Self {
            remaining: ready_timeout_sec.saturating_mul(2),
        }
    }

    pub fn remaining(self) -> u32 {
        self.remaining
    }

    pub fn is_exhausted(self) -> bool {
        self.remaining == 0
    }

    /// Charges one completed attempt. `gap` is the wall-clock time since the *previous* attempt
    /// began (`Duration::ZERO` for the first). A gap beyond [`SLEEP_GAP`] means the process was
    /// suspended, so the attempt is free.
    pub fn record(&mut self, gap: Duration) {
        if gap > SLEEP_GAP {
            return;
        }
        self.remaining = self.remaining.saturating_sub(1);
    }
}

/// Why the `starting` phase ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadyOutcome {
    /// The port answered on one of the two stacks.
    Ready,
    /// The child exited first — the exit watcher has already diagnosed and announced it.
    ChildExited,
    /// The attempt budget ran out with no answer.
    TimedOut,
}

/// SPEC.md §9 step 5: poll both stacks every 500 ms, **racing the child's exit**.
///
/// The race is the whole point. A command that dies in the first second — a typo, a missing
/// dependency, or the classic "user picked `build` instead of `dev`" — must be diagnosed in the
/// first second, not after sitting through the full 60 s timeout.
async fn await_ready(port: u16, ready_timeout_sec: u32, exited: &watch::Receiver<bool>) -> ReadyOutcome {
    let mut budget = AttemptBudget::new(ready_timeout_sec);
    let mut previous_attempt: Option<tokio::time::Instant> = None;

    loop {
        let started = tokio::time::Instant::now();
        if *exited.borrow() {
            return ReadyOutcome::ChildExited;
        }
        // Dual-stack, every attempt (§12 "Server bound to IPv6 localhost only").
        if process::port_accepts(port).await {
            return ReadyOutcome::Ready;
        }

        let gap = previous_attempt.map_or(Duration::ZERO, |prev| started.duration_since(prev));
        previous_attempt = Some(started);
        budget.record(gap);
        if budget.is_exhausted() {
            return ReadyOutcome::TimedOut;
        }

        // Sleeping *through* an exit is the bug this avoids: waiting on the reap signal wakes as
        // soon as the child is gone, and otherwise falls through on the normal 500 ms cadence.
        let mut rx = exited.clone();
        let _ = tokio::time::timeout(POLL_INTERVAL, rx.wait_for(|reaped| *reaped)).await;
    }
}

// ---------------------------------------------------------------------------------------------
// SPEC.md §9 steps 6-7 — the hand-off, and the timeout that must kill before it reports
// ---------------------------------------------------------------------------------------------

/// SPEC.md §9 step 7, as an ordering that cannot be got wrong by accident: the tree is killed and
/// its death confirmed **before** the status becomes `crashed`.
///
/// This is the fix for the spec's worst orphan bug. Invert it — or skip the kill — and a timed-out
/// server keeps running behind a `crashed` card that invites the user to Run again; the pre-check
/// then finds the pinned port free (the framework auto-bumped), a second tree spawns, and the first
/// becomes permanently untracked.
///
/// `kill` and `crash` are parameters so the ordering can be asserted without a live Tauri app; see
/// `the_timeout_kills_the_tree_before_it_says_crashed`.
async fn kill_then_crash<K, C, CF>(kill: K, crash: C) -> bool
where
    K: Future<Output = bool>,
    C: FnOnce(bool) -> CF,
    CF: Future<Output = ()>,
{
    let death_confirmed = kill.await;
    crash(death_confirmed).await;
    death_confirmed
}

/// SPEC.md §9 step 7's toast, verbatim, plus an honest note when the kill could not be verified.
pub fn timeout_message(port: u16, ready_timeout_sec: u32, death_confirmed: bool) -> String {
    let base = format!(
        "Server didn't answer on port {port} within {ready_timeout_sec} s, so it was stopped. If it \
         just needs longer (e.g. a first cold compile), raise Ready timeout in Edit. Check the log — \
         did it start on another port? Pin it in Edit."
    );
    if death_confirmed {
        base
    } else {
        // §8: never silently pretend it stopped. The status this message rides on is `crashed`,
        // where §6 refuses Stop and the card shows Run — a "press Stop to retry" would point at a
        // button that does not exist. What actually happens on the next click is `run_project`'s
        // §9 step 1 pre-check: it refuses to spawn while the port is still held and names the
        // process holding it (see `port_owner`), so that is what this tells the user to expect.
        format!(
            "{base} Some of its processes could not be confirmed dead. Run will refuse to start \
             while the port is still held and will name the process holding it."
        )
    }
}

/// SPEC.md §9 step 5's diagnosis for a child that exits while still `starting`, i.e. one that never
/// answered on the port.
///
/// The exit-0 case is the single most common user error in the whole app — picking `build` (or
/// `lint`, or `test`) in the Add dialog instead of a script that starts a server — and without this
/// message it presents as a silent, successful-looking crash.
pub fn starting_exit_message(exit_code: Option<i32>, command: &str, port: u16) -> String {
    match exit_code {
        Some(0) => format!(
            "`{command}` finished (exit 0) without ever answering on port {port} — did you pick a \
             script that starts a server (e.g. dev), not build?"
        ),
        Some(code) => format!(
            "`{command}` exited with code {code} without ever answering on port {port} — see the \
             log for the error."
        ),
        None => format!(
            "`{command}` was terminated without ever answering on port {port} — see the log."
        ),
    }
}

/// Picks the message for an observed child exit. Only an exit from `starting` gets the §9 step 5
/// diagnosis; an exit from `running` is an ordinary crash and keeps the exit-code note.
///
/// Called by the exit watcher, which is the one place that knows the exit happened — the §9 wording
/// lives here because §9 lives here.
pub async fn exit_message(
    app: &AppHandle,
    project_id: &str,
    from: Status,
    exit_code: Option<i32>,
    fallback: &str,
) -> String {
    if from != Status::Starting {
        return fallback.to_string();
    }
    let state = app.state::<AppState>();
    let project = {
        let projects = state.projects.lock().await;
        projects.iter().find(|p| p.id == project_id).cloned()
    };
    match project {
        Some(project) => starting_exit_message(exit_code, &project.command, project.port),
        None => fallback.to_string(),
    }
}

/// SPEC.md §5: `url` is an optional override; the default is computed from the pinned `port`.
/// Ready-check, busy-check and duplicate-port validation always use `port` regardless — this is
/// the *only* place `url` is honoured.
pub fn project_url(project: &Project) -> String {
    match project.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(url) => url.to_string(),
        None => format!("http://localhost:{}", project.port),
    }
}

/// SPEC.md §4: the opener plugin called **from Rust** bypasses the ACL and needs no capability
/// entry. Deliberately not routed through the webview, and never `tauri-plugin-shell` (§4 forbids
/// it, and its `open` is Tauri 2's deprecated path).
pub async fn open_in_browser(app: &AppHandle, project: &Project) -> Result<(), String> {
    let url = project_url(project);
    match app.opener().open_url(&url, None::<&str>) {
        Ok(()) => {
            process::append_system(app, &project.id, format!("opened {url}")).await;
            Ok(())
        }
        Err(e) => {
            let message = format!("Couldn't open {url}: {e}");
            process::append_system(app, &project.id, message.clone()).await;
            Err(message)
        }
    }
}

/// Bounded wait for the editor launcher's own exit (SPEC.md §7 `open_in_editor`). `code`/`subl`/etc.
/// hand off to an already-running instance (or fork a new one) and return almost immediately, so
/// this is generous without risking a wedged command hanging the toast.
const EDITOR_LAUNCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Shell-quotes an absolute path for the platform's spawn wrapper (SPEC.md §8: `/bin/sh -c` on
/// Unix, `cmd /C` on Windows) so a folder name containing a space cannot split it into two
/// arguments to the editor command.
#[cfg(unix)]
fn quote_for_shell(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

#[cfg(windows)]
fn quote_for_shell(path: &str) -> String {
    format!("\"{}\"", path.replace('"', "\"\""))
}

/// SPEC.md §7 `open_in_editor` / §10 step 7: `<editorCommand> <path>` through the ONE §8 spawn
/// helper — never a bare `Command`, because `code` is a `.cmd` batch shim on Windows that
/// `Command::new` cannot execute directly (§8's `cmd /C` wrapping is what makes it runnable).
///
/// Not registered as a kill target: the editor is not part of the project's process tree, and an
/// already-running VS Code just opens a new window and lets its own launcher process exit — this
/// awaits that exit (bounded by [`EDITOR_LAUNCH_TIMEOUT`]) so the child is still reaped rather than
/// abandoned (SPEC.md §8), without touching the project's own status or runtime entry.
pub async fn open_in_editor(app: &AppHandle, project: &Project) -> Result<(), String> {
    let state = app.state::<AppState>();
    let editor_command = state.settings.lock().await.editor_command.clone();
    let (env, path_searched) = {
        let dev_env = state.dev_env.get().await;
        (dev_env.vars.clone(), dev_env.effective_path())
    };

    let not_found = format!(
        "Couldn't run '{editor_command}' — is it on your PATH? Change the editor command in \
         Settings."
    );

    let spec = SpawnSpec {
        command: format!("{editor_command} {}", quote_for_shell(&project.path)),
        cwd: Some(PathBuf::from(&project.path)),
        env,
        extra_env: Vec::new(),
        long_lived: false,
        // Bounded by the timeout below rather than left to hang if the launcher is wedged.
        kill_on_drop: true,
        shell: ShellKind::Default,
    };

    let spawned = match process::spawn(&spec) {
        Ok(spawned) => spawned,
        Err(e) => return Err(format!("{not_found} ({e})\nPATH searched: {path_searched}")),
    };

    // SPEC.md §8/§12: `/bin/sh` (or `cmd`) always exists, so `spawn` above succeeds even when the
    // editor command does not — the shell reports "command not found" and exits 127 (see
    // `process::is_tool_not_found_exit`). That, not a spawn error, is the realistic failure this
    // toast exists for.
    match tokio::time::timeout(EDITOR_LAUNCH_TIMEOUT, spawned.child.wait_with_output()).await {
        Ok(Ok(output)) => match output.status.code() {
            Some(code) if process::is_tool_not_found_exit(code) => {
                Err(format!("{not_found}\nPATH searched: {path_searched}"))
            }
            _ => Ok(()),
        },
        // A launcher that errors out mid-wait or outlives the timeout has already done its job in
        // every realistic case (the editor itself detaches); `kill_on_drop` reaps whatever is left
        // rather than this leaving an abandoned `Child` handle (SPEC.md §8).
        Ok(Err(_)) | Err(_) => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------------
// SPEC.md §9 steps 2-3 — decide, once, which phases a Run actually performs
// ---------------------------------------------------------------------------------------------

/// SPEC.md §9 step 2 / §12. Only [`Pull`] enters `updating` at all — see `Trigger::Run`'s doc for
/// why the other two must not.
///
/// [`Pull`]: UpdatePlan::Pull
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdatePlan {
    Pull,
    /// §12: "git not found — skipping update" — always logged.
    GitMissing,
    /// `updateOnRun` is off, or the folder is not a git repo — §12: "Skip pull silently".
    Skip,
}

/// SPEC.md §9 step 2: decided once, before the run claims its first phase.
async fn plan_update(project: &Project, env: &EnvMap) -> UpdatePlan {
    if !project.update_on_run {
        return UpdatePlan::Skip;
    }
    match process::check_git_repo(Path::new(&project.path), env).await {
        process::GitAvailability::IsRepo => UpdatePlan::Pull,
        process::GitAvailability::GitMissing => UpdatePlan::GitMissing,
        process::GitAvailability::NotRepo => UpdatePlan::Skip,
    }
}

/// SPEC.md §9 step 3 / §12. Only [`Run`] enters `installing` — see `Trigger::Run`'s doc.
/// [`NoLockfile`]'s reason is always logged (unlike [`UpdatePlan::Skip`]'s silence): SPEC.md §9
/// step 3 gives it an explicit line even for the "no lockfile at all" case.
///
/// [`Run`]: InstallPlan::Run
/// [`NoLockfile`]: InstallPlan::NoLockfile
#[derive(Debug, Clone)]
enum InstallPlan {
    Run { kind: LockfileKind, hash: String },
    UpToDate,
    NoLockfile(String),
}

/// SPEC.md §9 step 3's three-way OR decision, decided once before the run claims its first phase
/// (or re-decided under the per-canonical-path mutex once a sibling project's install has been
/// awaited — see `run_project`'s `_path_guard`).
fn plan_install(project: &Project) -> InstallPlan {
    let dir = Path::new(&project.path);
    let Some((kind, lockfile_path)) = process::find_lockfile(dir) else {
        return InstallPlan::NoLockfile("no lockfile found — skipping install".to_string());
    };
    let hash = match process::hash_lockfile(&lockfile_path) {
        Ok(hash) => hash,
        Err(e) => {
            return InstallPlan::NoLockfile(format!(
                "could not hash the lockfile, skipping the install check: {e}"
            ))
        }
    };
    let node_modules_exists = dir.join("node_modules").is_dir();
    if process::needs_install(project.last_lockfile_hash.as_deref(), &hash, node_modules_exists) {
        InstallPlan::Run { kind, hash }
    } else {
        InstallPlan::UpToDate
    }
}

/// SPEC.md §9 steps 5-7, run in the background so `run_project` stays fire-and-forget (§7) instead
/// of holding an IPC call open for `readyTimeoutSec`.
async fn await_ready_then_hand_off(app: AppHandle, project: Project, exited: watch::Receiver<bool>) {
    match await_ready(project.port, project.ready_timeout_sec, &exited).await {
        // The exit watcher has already narrated the exit and applied `crashed` with the §9 step 5
        // diagnosis. Nothing to add, and nothing to kill.
        ReadyOutcome::ChildExited => {}

        ReadyOutcome::Ready => {
            tokio::time::sleep(READY_GRACE).await;
            // A Stop (or a crash) landing inside the grace wins: `Ready` is illegal from anything
            // but `starting`, so the rejection is the guard, and no tab is opened for a dead server.
            if apply(&app, &project.id, Trigger::Ready, None).await.is_err() {
                return;
            }
            let _ = open_in_browser(&app, &project).await;
        }

        ReadyOutcome::TimedOut => on_ready_timeout(&app, &project).await,
    }
}

/// SPEC.md §9 step 7. The ordering here is the point — see [`kill_then_crash`].
async fn on_ready_timeout(app: &AppHandle, project: &Project) {
    process::append_system(
        app,
        &project.id,
        format!(
            "no answer on port {} after {} poll attempts ({} s of polling) — stopping the process \
             tree before reporting the failure",
            project.port,
            // Attempts, not wall-clock: §9 step 5 counts attempts precisely so a system sleep does
            // not read as a slow server, and the log line should say what was actually measured.
            AttemptBudget::new(project.ready_timeout_sec).remaining(),
            project.ready_timeout_sec
        ),
    )
    .await;

    // Taken out of the map before the kill, exactly as `stop_project` does (SPEC.md §4: the async
    // mutex is never held across a multi-second kill). `claim_timeout_kill` is one call that both
    // sets the same flag Stop's `claim_stop` sets (SPEC.md §6's "someone else owns this exit's
    // announcement") and hands back the kill target — under this SAME lock acquisition, no window
    // between the two halves. With the flag set, the exit watcher's `observe_child_exit()` holds
    // its announcement instead of racing us to `crashed` with its own generic message, which
    // leaves the `Trigger::Failed` transition below real (`Starting` -> `Crashed`) — what makes it
    // emit `status-changed` and deliver the §9 step 7 toast. One call rather than two also closes
    // off a future edit that takes the target without claiming ownership first.
    //
    // Race invariant: if the child already exited and the exit watcher's lock acquisition happens
    // to land BEFORE this one, the flag is still false when it reads `observe_child_exit()` — it
    // has already applied `crashed` with its own diagnosis and announced it. Our `apply(...,
    // Trigger::Failed, ...)` below then finds `from == to == Crashed`, a documented no-op (§6:
    // "already settled") that emits nothing. That is fine: the watcher's diagnosis was accurate and
    // was delivered, so there is exactly one `crashed` event either way, never zero.
    let target = {
        let state = app.state::<AppState>();
        let mut runtime = state.runtime.lock().await;
        runtime
            .entry(project.id.clone())
            .or_default()
            .claim_timeout_kill()
    };

    kill_then_crash(
        async {
            let outcome = process::kill_tree(target).await;
            for note in outcome.notes {
                process::append_system(app, &project.id, note).await;
            }
            outcome.death_confirmed
        },
        |death_confirmed| async move {
            let message = timeout_message(project.port, project.ready_timeout_sec, death_confirmed);
            process::append_system(app, &project.id, message.clone()).await;
            let _ = apply(app, &project.id, Trigger::Failed, Some(message)).await;
        },
    )
    .await;
}

/// SPEC.md §9 steps 0, 1 and 4-7.
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
        let message = format!(
            "{} can't run: the folder {} no longer exists.",
            project.name, project.path
        );
        process::append_system(app, &project.id, message.clone()).await;
        return Err(message);
    }

    // ---- §9 step 1: the port must be free before anything is spawned --------------------------
    // Deliberately ahead of the claim: a refused Run must not flash `starting` and then a bogus
    // `crashed`. Racing Runs are still safe — the claim below is what is atomic, and the second one
    // is rejected there.
    if process::port_accepts(project.port).await {
        let env = state.dev_env.get().await.vars.clone();
        // Strictly read-only (§9): v0 has no button to kill this process. It is very often the
        // user's own terminal, and killing what Hangar did not spawn is exactly what §8 is careful
        // never to claim.
        let owner = process::port_owner(project.port, &env).await;
        let message = match owner {
            Some(owner) => format!(
                "Port {} is in use by {owner} — is this project running elsewhere?",
                project.port
            ),
            // The lookup failed, timed out, or found nothing: say less rather than guess.
            None => format!(
                "Port {} is already in use — is this project running elsewhere?",
                project.port
            ),
        };
        process::append_system(app, &project.id, message.clone()).await;
        return Err(message);
    }

    // ---- §6: fail a double-click fast, before touching the mutex below -------------------------
    // A real double-click must still be rejected near-instantly (§6: "impossible to double-spawn"),
    // not after waiting out a contended per-path mutex or a git-repo-check that a rejected Run will
    // throw away. This is a peek, not the atomic claim — `Status::Starting` is a placeholder target
    // that only matters if `from` turns out legal, in which case the real work below decides the
    // real one. The claim a few lines down is what actually enforces §6; a race that slips past this
    // peek is still caught there.
    let peeked_status = {
        let runtime = state.runtime.lock().await;
        runtime.get(&project.id).map(|e| e.status).unwrap_or(Status::Stopped)
    };
    if let Err(rejection) = next_status(peeked_status, Trigger::Run(Status::Starting)) {
        let message = rejection.for_project(&project.name);
        process::append_system(app, &project.id, message.clone()).await;
        return Err(message);
    }

    // ---- SPEC.md §9 step 3: serialize against any sibling project on the same folder -----------
    // Held across the phase decision below AND both phases that follow; dropped once `starting`
    // begins (SPEC.md §9 step 3: "steps 2-3 take a per-canonical-path mutex").
    let _path_guard = process::lock_project_path(Path::new(&project.path)).await;

    // ---- §8 environment resolution, needed early: the git-repo-check below spawns `git` ---------
    let (env, path_searched, notes) = {
        let dev_env = state.dev_env.get().await;
        (
            dev_env.vars.clone(),
            dev_env.effective_path(),
            dev_env.notes.clone(),
        )
    };

    // ---- SPEC.md §9 steps 2-3: decide, once, which phases this run actually performs ------------
    // Computed under the mutex (so a sibling's just-finished install is visible — the "re-check"
    // SPEC.md §9 step 3 asks for) and BEFORE the claim below, because §12's "skip pull silently" /
    // "no lockfile found — skipping install" both mean the corresponding status is never entered
    // at all — see `Trigger::Run`'s doc.
    let update_plan = plan_update(&project, &env).await;
    let mut install_plan = plan_install(&project);
    let first_phase = if update_plan == UpdatePlan::Pull {
        Status::Updating
    } else if matches!(install_plan, InstallPlan::Run { .. }) {
        Status::Installing
    } else {
        Status::Starting
    };

    // ---- §6 guard + claim, atomically ---------------------------------------------------------
    //
    // The claim moves the card into its first real phase, which is precisely what makes Stop legal
    // (§6). Every await below therefore runs inside a window where the user can press Stop but
    // there is no child to kill yet. `spawn_in_flight` is the run sequence's claim on that window: a
    // Stop arriving inside it parks on this receiver rather than verifying a death that has not
    // happened, and the run sequence cancels itself the moment it notices (SPEC.md §12, "Stop
    // clicked during updating/installing/starting").
    //
    // The sender is held for the whole body of this function: every return path drops it, which
    // releases a parked Stop even on a path that forgot to.
    let (spawn_claim, spawn_in_flight) = tokio::sync::watch::channel(false);
    apply_with(app, &project.id, Trigger::Run(first_phase), None, |entry| {
        entry.begin_run(spawn_in_flight)
    })
    .await
    .map_err(|rejection| rejection.for_project(&project.name))?;
    for note in notes {
        process::append_system(app, &project.id, note).await;
    }

    // ---- SPEC.md §9 steps 2-3: run whichever phases were actually planned above -----------------
    let mut spawn_registered = false;
    if let ControlFlow::Break(result) = advance_through_update(
        app,
        &project,
        &env,
        update_plan,
        &mut install_plan,
        &mut spawn_registered,
    )
    .await
    {
        return result;
    }
    if let ControlFlow::Break(result) = advance_through_install(
        app,
        &project,
        &env,
        &path_searched,
        &install_plan,
        &mut spawn_registered,
    )
    .await
    {
        return result;
    }
    drop(_path_guard);

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

    // ---- §9 step 4: spawn ---------------------------------------------------------------------
    //
    // Last look before anything is created (SPEC.md §9): a Stop that landed during the phases above
    // — or while `lastRunAt` was being persisted — costs zero *new* processes if it is noticed here.
    // `env`/`path_searched` were already resolved above (needed for the git-repo-check).
    if let Some(result) = bail_if_stop_pending(app, &project, spawn_registered).await {
        return result;
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
        // Cloned, not moved: the ready poller below races this same reap signal (§9 step 5), and
        // the runtime entry needs its own copy for the §8 kill path.
        entry.register_child(pid, exit_rx.clone(), path_searched.clone())
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

    // Started before the polling below, not after: it is what diagnoses an instantly-exiting
    // command, and it is what signals the reap the poller races.
    process::spawn_exit_watcher(app.clone(), project.id.clone(), child, pipeline, exit_tx);

    // §9 steps 5-7 run detached. `run_project` is fire-and-forget (§7) — holding the IPC call
    // open for up to `readyTimeoutSec` would block the frontend's next command behind a 60 s wait.
    tauri::async_runtime::spawn(await_ready_then_hand_off(
        app.clone(),
        project.clone(),
        exit_rx,
    ));

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

/// SPEC.md §9's "Last look before anything is created" guard, shared by every phase's spawn point
/// (git pull, installer, dev command). `spawn_registered` distinguishes the run's very first spawn
/// — where a parked Stop has nothing to signal yet, so this call must do the §8 kill+report itself,
/// exactly as the dev command's own pre-spawn check always has — from a later phase, where a
/// concurrent Stop already has (or, via the still-registered previous phase's kill primitive, will
/// get) a real target of its own; see `ProjectRuntime::claim_stop`. Calling `cancel_run` a second
/// time in that case would risk reporting `stopped` before the real kill has verified anything.
async fn bail_if_stop_pending(
    app: &AppHandle,
    project: &Project,
    spawn_registered: bool,
) -> Option<Result<(), String>> {
    if !stop_is_pending(app, &project.id).await {
        return None;
    }
    Some(if spawn_registered {
        Ok(())
    } else {
        cancel_run(app, project, KillTarget::default()).await
    })
}

/// What running one phase child (git pull, or an installer) produced.
#[derive(Debug)]
enum PhaseOutcome {
    /// Ran to completion under its own steam. `None` = terminated by signal / wait failed.
    Exited(Option<i32>),
    /// Could not even be spawned (SPEC.md §8/§12: `/bin/sh` itself always exists, so this is rare).
    SpawnFailed(String),
}

/// Spawns one phase child through the ONE §8 helper and wires it into the SAME kill bookkeeping
/// the dev command uses (`register_child`/`take_kill_target`), so Stop reaches it with no new
/// plumbing (SPEC.md §6: Stop is valid in every active phase). `Err(result)` means a Stop landed
/// before or during the spawn and `result` is what `run_project` must return as-is — see
/// `bail_if_stop_pending`'s doc for what `spawn_registered` distinguishes.
async fn run_phase_child(
    app: &AppHandle,
    project: &Project,
    spec: SpawnSpec,
    spawn_registered: &mut bool,
) -> Result<PhaseOutcome, Result<(), String>> {
    if let Some(result) = bail_if_stop_pending(app, project, *spawn_registered).await {
        return Err(result);
    }

    let spawned = match process::spawn(&spec) {
        Ok(spawned) => spawned,
        Err(e) => return Ok(PhaseOutcome::SpawnFailed(e)),
    };
    let mut child = spawned.child;
    let pid = child.id();
    let (exit_tx, exit_rx) = tokio::sync::watch::channel(false);

    let state = app.state::<AppState>();
    let outcome = {
        let mut runtime = state.runtime.lock().await;
        let entry = runtime.entry(project.id.clone()).or_default();
        #[cfg(windows)]
        {
            entry.job = spawned.job;
        }
        entry.register_child(pid, exit_rx.clone(), String::new())
    };
    *spawn_registered = true;

    let pipeline = process::attach_log_pipeline(app, &project.id, &mut child);
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    process::spawn_phase_reaper(app.clone(), project.id.clone(), child, pipeline, exit_tx, done_tx);

    if outcome == SpawnOutcome::CancelRun {
        let target = {
            let mut runtime = state.runtime.lock().await;
            runtime.entry(project.id.clone()).or_default().take_kill_target()
        };
        return Err(cancel_run(app, project, target).await);
    }

    match done_rx.await {
        Ok(code) => Ok(PhaseOutcome::Exited(code)),
        // The reaper task itself was dropped/panicked before sending — treat as an unknown exit
        // rather than propagating a channel error nobody asked for.
        Err(_) => Ok(PhaseOutcome::Exited(None)),
    }
}

/// SPEC.md §9 step 3: an install failure crashes the run (unlike a pull failure, which never
/// does). Applies the §6 transition and hands back the `Err` `run_project` returns.
async fn crash_run(app: &AppHandle, project_id: &str, message: String) -> ControlFlow<Result<(), String>> {
    let _ = apply_with(app, project_id, Trigger::Failed, Some(message.clone()), |_| {}).await;
    ControlFlow::Break(Err(message))
}

/// SPEC.md §9 step 3: "Store the new hash only after success." Same snapshot-then-save shape as
/// the `lastRunAt` write in `run_project` (plan 010's maintenance note: new registry writers
/// should follow it).
async fn store_lockfile_hash(app: &AppHandle, project: &Project, hash: &str) {
    let state = app.state::<AppState>();
    let persist_error = {
        let mut projects = state.projects.lock().await;
        if let Some(p) = projects.iter_mut().find(|p| p.id == project.id) {
            p.last_lockfile_hash = Some(hash.to_string());
        }
        registry::save_projects(&state.config_dir, &projects).err()
    };
    if let Some(e) = persist_error {
        process::append_system(app, &project.id, format!("could not save the lockfile hash: {e}"))
            .await;
    }
}

/// SPEC.md §9 step 2: "10 s timeout; on timeout kill the git tree — git spawns ssh and
/// credential-helper children."
const GIT_PULL_TIMEOUT: Duration = Duration::from_secs(10);

/// SPEC.md §9 step 2: "auth must fail fast, never prompt" — all four non-interactive variables.
fn git_pull_env() -> Vec<(String, String)> {
    vec![
        ("GIT_TERMINAL_PROMPT".to_string(), "0".to_string()),
        ("GIT_ASKPASS".to_string(), "echo".to_string()),
        ("GIT_SSH_COMMAND".to_string(), "ssh -oBatchMode=yes".to_string()),
        ("GCM_INTERACTIVE".to_string(), "never".to_string()),
    ]
}

/// SPEC.md §9 step 2's timeout branch: tree-kill via the same §8 path `on_ready_timeout` uses. The
/// primitive is still registered even though `run_phase_child`'s future was just dropped by the
/// `tokio::time::timeout` that raced it — `spawn_phase_reaper` runs detached and keeps reaping.
async fn kill_timed_out_pull(app: &AppHandle, project: &Project) {
    let target = {
        let state = app.state::<AppState>();
        let mut runtime = state.runtime.lock().await;
        runtime.entry(project.id.clone()).or_default().take_kill_target()
    };
    let outcome = process::kill_tree(target).await;
    for note in outcome.notes {
        process::append_system(app, &project.id, note).await;
    }
    process::append_system(
        app,
        &project.id,
        format!(
            "git pull timed out after {} s — stopped it and continuing without updating",
            GIT_PULL_TIMEOUT.as_secs()
        ),
    )
    .await;
}

/// SPEC.md §9 step 2: "On any failure … write a warning to the log and continue anyway." A failure
/// mentioning `index.lock` gets a named hint — SPEC.md: "never delete it automatically".
async fn warn_pull_failure(app: &AppHandle, project_id: &str, exit_code: Option<i32>) {
    let message = match exit_code {
        Some(code) => format!("git pull failed (exit {code}) — continuing without updating"),
        None => "git pull was terminated — continuing without updating".to_string(),
    };
    process::append_system(app, project_id, message).await;

    let mentions_lock = {
        let state = app.state::<AppState>();
        let runtime = state.runtime.lock().await;
        runtime.get(project_id).is_some_and(|entry| {
            entry.logs.snapshot().iter().any(|l| l.line.contains("index.lock"))
        })
    };
    if mentions_lock {
        process::append_system(
            app,
            project_id,
            "the pull output mentioned index.lock — another git process may be using this repo; \
             Hangar will not delete it automatically",
        )
        .await;
    }
}

/// SPEC.md §9 step 2, run once `entry.status` is already `updating` (the caller claimed
/// `Trigger::Run(Status::Updating)` because `plan` was already known to be [`UpdatePlan::Pull`]).
/// Advances to whichever phase comes next when done — a pull failure or timeout warns and
/// continues (SPEC.md §9 step 2), it never crashes the run, so this never returns `Break` for that
/// reason; it only does when a Stop has claimed the outcome.
///
/// `install_plan` is re-decided here, in place, after the pull attempt — a successful `git pull`
/// can itself change the lockfile, so the decision made before the pull (used only to pick
/// `first_phase`) would otherwise go stale and could wrongly skip an install the pull just made
/// necessary.
async fn advance_through_update(
    app: &AppHandle,
    project: &Project,
    env: &EnvMap,
    plan: UpdatePlan,
    install_plan: &mut InstallPlan,
    spawn_registered: &mut bool,
) -> ControlFlow<Result<(), String>> {
    if plan == UpdatePlan::GitMissing {
        process::append_system(app, &project.id, "git not found — skipping update").await;
    }
    if plan != UpdatePlan::Pull {
        return ControlFlow::Continue(());
    }

    let spec = SpawnSpec {
        command: "git pull --ff-only".to_string(),
        cwd: Some(PathBuf::from(&project.path)),
        env: env.clone(),
        extra_env: git_pull_env(),
        long_lived: true,
        kill_on_drop: false,
        shell: ShellKind::Default,
    };

    match tokio::time::timeout(GIT_PULL_TIMEOUT, run_phase_child(app, project, spec, spawn_registered))
        .await
    {
        Ok(Err(result)) => return ControlFlow::Break(result),
        Ok(Ok(PhaseOutcome::Exited(Some(0)))) => {}
        Ok(Ok(PhaseOutcome::Exited(code))) => warn_pull_failure(app, &project.id, code).await,
        Ok(Ok(PhaseOutcome::SpawnFailed(e))) => {
            process::append_system(
                app,
                &project.id,
                format!("could not start git pull: {e} — continuing without updating"),
            )
            .await;
        }
        Err(_elapsed) => kill_timed_out_pull(app, project).await,
    }

    if let Some(result) = bail_if_stop_pending(app, project, *spawn_registered).await {
        return ControlFlow::Break(result);
    }
    // Re-decide with fresh eyes: the pull we just ran may have changed the lockfile.
    *install_plan = plan_install(project);
    let next = if matches!(install_plan, InstallPlan::Run { .. }) {
        Status::Installing
    } else {
        Status::Starting
    };
    let _ = apply(app, &project.id, Trigger::PhaseAdvance(next), None).await;
    ControlFlow::Continue(())
}

/// SPEC.md §9 step 3, run once `entry.status` is already `installing` (the caller claimed
/// `Trigger::Run(Status::Installing)`, or just advanced into it, because `plan` was already known
/// to be [`InstallPlan::Run`]). Unlike the update phase, a genuine install failure DOES crash the
/// run (SPEC.md §9 step 3) — see `crash_run`.
async fn advance_through_install(
    app: &AppHandle,
    project: &Project,
    env: &EnvMap,
    path_searched: &str,
    plan: &InstallPlan,
    spawn_registered: &mut bool,
) -> ControlFlow<Result<(), String>> {
    let InstallPlan::Run { kind, hash } = plan else {
        if let InstallPlan::NoLockfile(reason) = plan {
            process::append_system(app, &project.id, reason.clone()).await;
        }
        return ControlFlow::Continue(());
    };

    let spec = SpawnSpec {
        command: kind.install_command().to_string(),
        cwd: Some(PathBuf::from(&project.path)),
        env: env.clone(),
        extra_env: Vec::new(),
        long_lived: true,
        kill_on_drop: false,
        shell: ShellKind::Default,
    };

    let outcome = match run_phase_child(app, project, spec, spawn_registered).await {
        Ok(outcome) => outcome,
        Err(result) => {
            process::append_system(
                app,
                &project.id,
                "install was stopped — node_modules may be partial",
            )
            .await;
            return ControlFlow::Break(result);
        }
    };

    match outcome {
        PhaseOutcome::Exited(Some(0)) => store_lockfile_hash(app, project, hash).await,
        PhaseOutcome::Exited(code) => {
            if code.is_some_and(process::is_tool_not_found_exit) {
                process::append_system(app, &project.id, format!("PATH searched: {path_searched}"))
                    .await;
            }
            let message = match code {
                Some(n) => format!("Install failed (exit {n}) — see the log, then Run again."),
                None => "Install failed — see the log, then Run again.".to_string(),
            };
            return crash_run(app, &project.id, message).await;
        }
        PhaseOutcome::SpawnFailed(e) => {
            return crash_run(
                app,
                &project.id,
                format!("Install failed to start: {e} — see the log, then Run again."),
            )
            .await;
        }
    }

    if let Some(result) = bail_if_stop_pending(app, project, *spawn_registered).await {
        return ControlFlow::Break(result);
    }
    let _ = apply(app, &project.id, Trigger::PhaseAdvance(Status::Starting), None).await;
    ControlFlow::Continue(())
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
        for first_phase in [Status::Updating, Status::Installing, Status::Starting] {
            assert_eq!(
                next_status(Status::Stopped, Trigger::Run(first_phase)),
                Ok(first_phase)
            );
            assert_eq!(
                next_status(Status::Crashed, Trigger::Run(first_phase)),
                Ok(first_phase)
            );
        }

        for from in EVERY_STATUS {
            if matches!(from, Status::Stopped | Status::Crashed) {
                continue;
            }
            assert!(
                next_status(from, Trigger::Run(Status::Starting)).is_err(),
                "Run must be rejected from {}",
                status_label(from)
            );
        }
    }

    #[test]
    fn run_into_a_non_phase_status_is_rejected_even_from_stopped() {
        // The payload is trusted to be one of the three real phases everywhere else; this is the
        // one place that asserts the guard actually rejects a malformed one rather than silently
        // accepting it.
        for bogus in [Status::Running, Status::Stopping, Status::Crashed, Status::StopFailed] {
            assert!(next_status(Status::Stopped, Trigger::Run(bogus)).is_err());
        }
    }

    #[test]
    fn run_is_rejected_from_running() {
        // The double-click case, called out explicitly: it must be impossible to double-spawn.
        let rejection = next_status(Status::Running, Trigger::Run(Status::Starting)).unwrap_err();
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

    // -----------------------------------------------------------------------------------------
    // SPEC.md §9 step 5 — the attempt-counted, sleep-proof ready budget
    // -----------------------------------------------------------------------------------------

    #[test]
    fn the_budget_is_two_attempts_per_second_of_ready_timeout() {
        assert_eq!(AttemptBudget::new(60).remaining(), 120, "the §5 default");
        assert_eq!(AttemptBudget::new(1).remaining(), 2);
        assert_eq!(AttemptBudget::new(0).remaining(), 0);
    }

    #[test]
    fn each_completed_attempt_spends_exactly_one() {
        let mut budget = AttemptBudget::new(2); // 4 attempts
        for expected in [3, 2, 1, 0] {
            budget.record(POLL_INTERVAL);
            assert_eq!(budget.remaining(), expected);
        }
        assert!(budget.is_exhausted());

        // Exhausted stays exhausted; it must never wrap around into a fresh budget.
        budget.record(POLL_INTERVAL);
        assert_eq!(budget.remaining(), 0);
    }

    #[test]
    fn a_system_sleep_does_not_consume_the_budget() {
        // SPEC.md §12, "System sleep during `starting`": the laptop was shut for two hours. A
        // wall-clock deadline would have expired 7199 s ago and killed a healthy server the moment
        // the machine woke; the attempt count is untouched because no attempt was actually made.
        let mut budget = AttemptBudget::new(60);
        for _ in 0..100 {
            budget.record(Duration::from_secs(7200));
        }
        assert_eq!(budget.remaining(), 120, "a suspended machine is not a slow server");

        // The boundary: 5 s exactly is still a (very slow) poll and is charged; beyond it is sleep.
        let mut budget = AttemptBudget::new(1);
        budget.record(SLEEP_GAP);
        assert_eq!(budget.remaining(), 1);
        budget.record(SLEEP_GAP + Duration::from_millis(1));
        assert_eq!(budget.remaining(), 1);
    }

    // -----------------------------------------------------------------------------------------
    // SPEC.md §9 step 7 — kill the tree BEFORE saying `crashed`
    // -----------------------------------------------------------------------------------------

    #[test]
    fn the_timeout_kills_the_tree_before_it_says_crashed() {
        use std::sync::{Arc, Mutex};

        let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
        let kill_order = Arc::clone(&order);
        let crash_order = Arc::clone(&order);

        // `block_on` uses Tauri's runtime — SPEC.md §4 forbids creating one of our own.
        let confirmed = tauri::async_runtime::block_on(kill_then_crash(
            async move {
                kill_order.lock().unwrap().push("kill");
                true
            },
            move |death_confirmed| async move {
                crash_order
                    .lock()
                    .unwrap()
                    .push(if death_confirmed { "crash" } else { "crash-unverified" });
            },
        ));

        assert!(confirmed);
        assert_eq!(
            *order.lock().unwrap(),
            ["kill", "crash"],
            "inverting this order is the orphan bug SPEC.md §9 step 7 exists to close"
        );
    }

    #[test]
    fn a_timeout_whose_kill_could_not_be_verified_says_so() {
        let verified = timeout_message(3000, 60, true);
        assert!(verified.starts_with(
            "Server didn't answer on port 3000 within 60 s, so it was stopped."
        ));
        assert!(verified.contains("raise Ready timeout in Edit"));
        assert!(verified.contains("did it start on another port? Pin it in Edit."));
        assert!(!verified.contains("press Stop to retry"));

        // §8: never silently pretend it stopped. Plan 007: the old wording ("press Stop to retry")
        // pointed at a button that does not exist — the status here is `crashed`, where §6 refuses
        // Stop and the card shows Run. The replacement names what Run's own §9 step 1 pre-check
        // actually does.
        let unverified = timeout_message(3000, 60, false);
        assert!(unverified.contains(
            "Run will refuse to start while the port is still held and will name the process \
             holding it."
        ));
        assert!(!unverified.contains("press Stop to retry"));
    }

    /// Plan 007: the ownership mechanics that make the §9 step 7 toast reach the user, at the
    /// state-machine level — no live Tauri app needed, same style as
    /// `the_timeout_kills_the_tree_before_it_says_crashed`.
    ///
    /// With `claim_timeout_kill` called (mirrored here as `ChildExit { user_stop: true }`, the
    /// same flag `claim_stop` sets), the exit watcher holds `Starting` instead of announcing —
    /// leaving the timeout path's own `Failed` trigger to fire a REAL `Starting -> Crashed`
    /// transition, which is what makes `apply_with` emit `status-changed` at all. The structural
    /// guard that this can't be dropped by a future edit lives in process.rs's
    /// `a_timeout_kill_claim_holds_the_exit_watcher`; this test documents the §6 rows it relies on.
    #[test]
    fn claiming_exit_ownership_holds_the_watcher_so_the_timeout_transition_is_real() {
        // The exit watcher's half: with the flag set, it announces nothing and `Starting` holds.
        assert_eq!(
            next_status(Status::Starting, Trigger::ChildExit { user_stop: true }),
            Ok(Status::Starting)
        );
        // The timeout path's half: its own `Failed` trigger is then a real, emitting transition.
        assert_eq!(
            next_status(Status::Starting, Trigger::Failed),
            Ok(Status::Crashed)
        );
    }

    /// The documented loser of the race (Plan 007 step 2): if the exit watcher's lock acquisition
    /// lands before the timeout path claims ownership, the watcher has already diagnosed and
    /// announced the exit — the timeout path's later `Failed` trigger is then `Crashed -> Crashed`,
    /// a no-op (`from == to`) that `apply_with` deliberately does not emit. Pinned here as the
    /// accepted, documented no-toast case: exactly one `crashed` event either way, never zero.
    #[test]
    fn without_ownership_the_watchers_announcement_stands_and_the_timeout_transition_is_silent() {
        assert_eq!(
            next_status(Status::Starting, Trigger::ChildExit { user_stop: false }),
            Ok(Status::Crashed)
        );
        assert_eq!(next_status(Status::Crashed, Trigger::Failed), Ok(Status::Crashed));
    }

    // -----------------------------------------------------------------------------------------
    // SPEC.md §9 step 5 — diagnosing a child that exits while still `starting`
    // -----------------------------------------------------------------------------------------

    #[test]
    fn an_exit_zero_while_starting_asks_about_the_wrong_script() {
        // §12: "Child exits during `starting` (exit 0 — e.g. user picked `build`)".
        let message = starting_exit_message(Some(0), "npm run build", 3000);
        assert_eq!(
            message,
            "`npm run build` finished (exit 0) without ever answering on port 3000 — did you pick \
             a script that starts a server (e.g. dev), not build?"
        );
    }

    #[test]
    fn a_nonzero_exit_while_starting_reports_the_code_and_points_at_the_log() {
        let message = starting_exit_message(Some(1), "npm run dev", 5173);
        assert!(message.contains("exited with code 1"), "got {message}");
        assert!(message.contains("port 5173"), "got {message}");
        assert!(message.contains("see the log"), "got {message}");
        assert!(
            !message.contains("did you pick"),
            "a real failure must not be explained away as the wrong script: {message}"
        );

        // Killed by a signal: no code to report, but still not a mystery.
        let signalled = starting_exit_message(None, "npm run dev", 5173);
        assert!(signalled.contains("was terminated"), "got {signalled}");
    }

    // -----------------------------------------------------------------------------------------
    // SPEC.md §5 — `url` is an override for the browser only
    // -----------------------------------------------------------------------------------------

    fn project_fixture(url: Option<&str>) -> Project {
        Project {
            id: "ielts-coach".into(),
            name: "IELTS Coach".into(),
            path: "/tmp/ielts".into(),
            command: "npm run dev".into(),
            port: 3000,
            url: url.map(str::to_string),
            update_on_run: true,
            ready_timeout_sec: 60,
            last_lockfile_hash: None,
            last_run_at: None,
            notes: None,
        }
    }

    #[test]
    fn the_browser_url_defaults_to_the_pinned_port() {
        assert_eq!(project_url(&project_fixture(None)), "http://localhost:3000");
    }

    #[test]
    fn plan_install_re_reads_after_a_sibling_projects_install_lands() {
        // SPEC.md §9 step 3: "the project that went first has usually already installed, so the
        // second should skip". The mechanism is simply that `plan_install` never caches — it
        // re-reads the hash and `node_modules` fresh on every call, which is what makes the
        // per-canonical-path mutex's "re-check after acquiring it" meaningful.
        let dir = std::env::temp_dir().join(format!(
            "hangar-plan-install-test-{}",
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();
        let hash = process::hash_lockfile(&dir.join("package-lock.json")).unwrap();

        let mut project = project_fixture(None);
        project.path = dir.to_string_lossy().into_owned();
        project.last_lockfile_hash = Some(hash);

        // Before the sibling's install: node_modules is missing, so this project still needs one.
        assert!(matches!(plan_install(&project), InstallPlan::Run { .. }));

        // The sibling project's install lands.
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();

        // Re-checked: same project, same lockfile hash, but node_modules exists now — up to date.
        assert!(matches!(plan_install(&project), InstallPlan::UpToDate));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_url_override_is_honoured_but_blank_is_not() {
        assert_eq!(
            project_url(&project_fixture(Some("http://localhost:3000/dashboard"))),
            "http://localhost:3000/dashboard"
        );
        // An empty or whitespace override is a leftover from the Edit dialog, not an instruction.
        assert_eq!(project_url(&project_fixture(Some(""))), "http://localhost:3000");
        assert_eq!(project_url(&project_fixture(Some("   "))), "http://localhost:3000");
    }

    // -----------------------------------------------------------------------------------------
    // SPEC.md §15 acceptance tests, against the real code paths.
    //
    // Ignored by default: they start real `node` processes. Run them with
    //
    //     cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture --test-threads=1
    //
    // `--test-threads=1` is REQUIRED, not tidiness. SPEC.md §15's measurement is `pgrep -f node`,
    // which is a machine-wide count: run these concurrently and each one's fixture lands inside the
    // others' before/after window, so they fail with the count going *down*. That reads exactly like
    // an orphan bug and is not one. Same reason §15 test 3 says to take the baseline after a prior
    // Run — the count only means something when nothing else is moving.
    // -----------------------------------------------------------------------------------------

    /// Writes a throwaway Node project and returns its directory.
    #[cfg(unix)]
    fn write_fixture(tag: &str, server_js: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hangar-{tag}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create the fixture directory");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"hangar-fixture","private":true,"version":"0.0.0","scripts":{"dev":"node server.js"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("server.js"), server_js).unwrap();
        dir
    }

    #[cfg(unix)]
    async fn count_node_processes() -> usize {
        let spec = SpawnSpec {
            command: "pgrep -f node | wc -l".to_string(),
            kill_on_drop: true,
            ..SpawnSpec::default()
        };
        let spawned = process::spawn(&spec).expect("spawn pgrep");
        let output = spawned.child.wait_with_output().await.expect("run pgrep");
        String::from_utf8_lossy(&output.stdout).trim().parse().unwrap_or(0)
    }

    /// SPEC.md §15 test 7 — **the timeout-orphan test**, and the reason §9 step 7 orders the kill
    /// before the status change.
    ///
    /// The fixture reproduces the exact failure: it binds a *different* port from the pinned one,
    /// which is what a framework does when it finds the pinned port busy and auto-bumps. The
    /// ready-check can therefore never succeed. Before the fix the card went `crashed` while the
    /// server kept running, and the next Run — finding the pinned port free — spawned a second tree
    /// and orphaned the first forever.
    ///
    /// It also spawns a SIGTERM-ignoring grandchild that never listens on any port, so a port-only
    /// verification would call this a clean stop while it was still running.
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn a_ready_timeout_kills_the_tree_and_leaves_no_orphans() {
        const PINNED: u16 = 39221; // nothing ever listens here — the auto-bump case
        const ACTUAL: u16 = 39222; // where the fixture really binds

        let dir = write_fixture(
            "timeout-orphan",
            r#"const http = require('http');
const { spawn } = require('child_process');
spawn(process.execPath, ['-e', "process.on('SIGTERM', () => {}); setInterval(() => {}, 1000)"], {
  stdio: 'ignore',
});
http.createServer((_req, res) => res.end('ok')).listen(39222, () => console.log('listening'));
"#,
        );

        let fixture = dir.clone();
        tauri::async_runtime::block_on(async move {
            let before = count_node_processes().await;

            let spawned = process::spawn(&SpawnSpec {
                command: "npm run dev".to_string(),
                cwd: Some(fixture),
                long_lived: true,
                ..SpawnSpec::default()
            })
            .expect("spawn npm run dev");
            let mut child = spawned.child;
            let pid = child.id().expect("the child has a pid");

            // The production contract: one task owns `wait()` and signals the kill path.
            let (exit_tx, exit_rx) = watch::channel(false);
            tauri::async_runtime::spawn(async move {
                let _ = child.wait().await;
                let _ = exit_tx.send(true);
            });

            // It really is up on its bumped port, so this is a live server being timed out.
            let mut up = false;
            for _ in 0..60 {
                if process::port_accepts(ACTUAL).await {
                    up = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            assert!(up, "the fixture never bound its own port {ACTUAL}");

            let during = count_node_processes().await;
            println!("timeout-orphan: node before={before} during={during}");
            assert!(during > before, "the fixture must create node processes");

            // §9 step 5: 2 s of budget on a port nobody answers.
            let outcome = await_ready(PINNED, 2, &exit_rx).await;
            assert_eq!(
                outcome,
                ReadyOutcome::TimedOut,
                "the pinned port never answers, so this must time out rather than go ready"
            );

            // §9 step 7: kill FIRST, and only a confirmed death may be reported.
            let kill = process::kill_tree(process::KillTarget {
                pid: Some(pid),
                exited: Some(exit_rx),
                ..process::KillTarget::default()
            })
            .await;
            for note in &kill.notes {
                println!("timeout-orphan: {note}");
            }
            assert!(kill.death_confirmed, "the timed-out tree was not confirmed dead");

            let after = count_node_processes().await;
            println!("timeout-orphan: node after={after}");
            assert_eq!(after, before, "a timed-out tree must not be left running");
            assert!(
                !process::port_accepts(ACTUAL).await,
                "the bumped port {ACTUAL} still answers — the server outlived its own timeout"
            );
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC.md §15 test 4 (bonus) and §12's "Child exits during `starting`" rows: a command that
    /// exits instantly must be diagnosed instantly.
    ///
    /// This is what makes picking `build` instead of `dev` a one-second mistake rather than a
    /// 60-second mystery — the poll loop races the child's exit instead of sleeping through it.
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn a_command_that_exits_at_once_is_diagnosed_at_once_not_after_the_timeout() {
        const PINNED: u16 = 39223;

        tauri::async_runtime::block_on(async {
            let spawned = process::spawn(&SpawnSpec {
                // Stands in for `npm run build`: does its job, exits 0, never serves anything.
                command: "node -e 'process.exit(0)'".to_string(),
                long_lived: true,
                ..SpawnSpec::default()
            })
            .expect("spawn the instant-exit command");
            let mut child = spawned.child;

            let (exit_tx, exit_rx) = watch::channel(false);
            tauri::async_runtime::spawn(async move {
                let _ = child.wait().await;
                let _ = exit_tx.send(true);
            });

            // A 60 s ready timeout — the §5 default. If the loop slept through the exit this would
            // take a minute.
            let started = tokio::time::Instant::now();
            let outcome = await_ready(PINNED, 60, &exit_rx).await;
            let elapsed = started.elapsed();

            println!("instant-exit: {outcome:?} after {} ms", elapsed.as_millis());
            assert_eq!(outcome, ReadyOutcome::ChildExited);
            assert!(
                elapsed < Duration::from_secs(5),
                "the exit must be noticed at once, not after the 60 s budget — took {elapsed:?}"
            );
        });
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
