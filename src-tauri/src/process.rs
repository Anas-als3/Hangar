//! SPEC.md §8 — the ONE shared spawn helper, the platform kill paths (Job Objects on Windows),
//! the line-oriented log reader, and the per-project 500-line ring buffers.
//!
//! Plan 002 (M2) implemented the spawn side, the log pipeline and the exit watcher. Plan 003 (M3)
//! adds the other half: the dual-stack port probe, `TerminateJobObject` / `SIGTERM`-then-`SIGKILL`
//! to the process group, and the death-then-port verification §8 requires.
//!
//! This module owns the *mechanics*. Which status those mechanics produce is SPEC.md §6, and lives
//! in one place only: `run::next_status` and `run::apply*`.
//!
//! **Every** child process Hangar will ever spawn goes through [`spawn`] — that is the only way the
//! Windows flags (`raw_arg`, `CREATE_NO_WINDOW`, Job Object assignment) and the universal
//! `stdin: null` cannot be forgotten by a later plan. No `Command` may be constructed anywhere else.

use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex as StdMutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;

use crate::commands::AppState;
use crate::env_resolve::EnvMap;
use crate::registry::Status;

/// SPEC.md §8 log pipeline constants. These are requirements, not tuning knobs.
pub const RING_CAPACITY: usize = 500;
pub const MAX_LINE_BYTES: usize = 4096;
pub const TRUNCATION_MARKER: &str = " …[truncated]";
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(100);
pub const MAX_LINES_PER_FLUSH: usize = 2000;

/// `CREATE_NO_WINDOW` — set on EVERY Windows spawn, helpers included, so git/taskkill never flash
/// a console window (SPEC.md §8).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// SPEC.md §8 kill constants. Requirements, not tuning knobs.
///
/// `TERM_GRACE`: "SIGTERM to -pgid, wait up to 5 s racing `child.wait()`, then SIGKILL". Unix only —
/// `TerminateJobObject` is atomic and has nothing to wait for.
/// `DEATH_CONFIRM_TIMEOUT`: "`kill(-pgid, 0)` returns ESRCH (poll up to 3 s)".
#[cfg(unix)]
pub const TERM_GRACE: Duration = Duration::from_secs(5);
pub const DEATH_CONFIRM_TIMEOUT: Duration = Duration::from_secs(3);
pub const DEATH_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// The direct child must be *reaped*, never abandoned (§8) — but a wedged `wait()` must not hang
/// Stop forever either, so the wait for the exit watcher is bounded.
pub const REAP_TIMEOUT: Duration = Duration::from_secs(5);
/// Per-attempt TCP connect budget for the dual-stack probe. Loopback answers in microseconds; this
/// only has to cover a machine under heavy load.
pub const PORT_PROBE_TIMEOUT: Duration = Duration::from_millis(400);

// ---------------------------------------------------------------------------------------------
// Log lines and the per-project ring buffer
// ---------------------------------------------------------------------------------------------

/// SPEC.md §7 `LogLine.stream`. `system` is Hangar's own narration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
    System,
}

/// SPEC.md §7 — mirrored by `LogLine` in `src/types.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub stream: Stream,
    pub line: String,
}

impl LogLine {
    pub fn system(line: impl Into<String>) -> Self {
        Self {
            stream: Stream::System,
            line: line.into(),
        }
    }
}

/// SPEC.md §8: "Rust owns the buffer" — the last 500 lines per project, appended BEFORE any event
/// is emitted, cleared at the start of each Run and retained after exit/crash/stop.
#[derive(Debug, Default)]
pub struct LogBuffer {
    lines: VecDeque<LogLine>,
}

impl LogBuffer {
    pub fn push(&mut self, line: LogLine) {
        if self.lines.len() == RING_CAPACITY {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn snapshot(&self) -> Vec<LogLine> {
        self.lines.iter().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

// ---------------------------------------------------------------------------------------------
// Per-project runtime state (managed state — SPEC.md §4)
// ---------------------------------------------------------------------------------------------

/// Everything Hangar knows about a project that is NOT persisted in `projects.json`.
#[derive(Debug)]
pub struct ProjectRuntime {
    pub status: Status,
    pub logs: LogBuffer,
    /// SPEC.md §6: set by Stop **before the kill begins**, cleared at the start of each Run. The
    /// exit watcher reads it to decide `crashed` vs "a Stop is in flight and owns the outcome".
    pub user_stop: bool,
    /// The kill primitive for the current run. On Unix this is the process-group id (the child is
    /// spawned with `.process_group(0)`, so its pid *is* the pgid); on Windows it is the direct
    /// child's pid, used only by the `taskkill` fallback.
    ///
    /// It deliberately **survives the reap**. The exit watcher must never clear it: a Stop that
    /// reaches `stop-failed` has already reaped its direct child, and a retry Stop (SPEC.md §6,
    /// row "`stop-failed` | Stop clicked | `stopping` — retry the kill") with nothing to signal is
    /// not a retry at all — it falls through to the port probe, which the surviving children
    /// (esbuild service, file watchers) never answer, and announces a stop that never happened.
    /// Cleared in exactly two places: a *verified* stop, and the start of the next Run.
    pub kill_pid: Option<u32>,
    /// True from the moment a child is registered for the current run until that run's tree is
    /// confirmed dead (or the next Run claims the project). It is what tells "Stop pressed with no
    /// child of ours anywhere" apart from "the primitive was lost", so [`kill_tree`]'s
    /// nothing-to-signal early return can never launder a previous failure into a confirmed death.
    pub child_registered: bool,
    /// Held by the run sequence for the whole window between its §6 `Run` claim — which is what
    /// makes Stop legal — and the moment its child is finally registered above. Several awaits live
    /// inside that window (the `lastRunAt` write, the login-shell environment resolution, plans
    /// 004/006's pull and install phases), and a Stop landing there has nothing to signal *yet*.
    /// It waits on this instead of verifying a death that has not happened; the run sequence
    /// observes `user_stop` on the other side and cancels itself.
    pub spawn_in_flight: Option<watch::Receiver<bool>>,
    /// Flips to `true` when the exit watcher has reaped the child (SPEC.md §8: "every kill path ends
    /// by awaiting the same wait future"). The kill sequence cannot call `child.wait()` itself —
    /// the watcher owns the `Child` — so it awaits this instead.
    pub exited: Option<watch::Receiver<bool>>,
    /// The PATH the current child was actually given (`DevEnvironment::effective_path()`), recorded
    /// at spawn so the exit watcher can print it when the shell exits "command not found"
    /// (SPEC.md §8 and the §12 nvm-from-Dock row). See [`is_tool_not_found_exit`].
    pub path_searched: Option<String>,
    /// The Job Object the child was assigned to at spawn. `None` means assignment failed and the
    /// kill must fall back to `taskkill /PID <pid> /T /F` (SPEC.md §8). Like `kill_pid` it survives
    /// the reap, and a failed verification hands it back so the retry still owns it.
    #[cfg(windows)]
    pub job: Option<win32job::Job>,
}

impl Default for ProjectRuntime {
    fn default() -> Self {
        Self {
            status: Status::Stopped,
            logs: LogBuffer::default(),
            user_stop: false,
            kill_pid: None,
            child_registered: false,
            spawn_in_flight: None,
            exited: None,
            path_searched: None,
            #[cfg(windows)]
            job: None,
        }
    }
}

/// What the run sequence must do at the moment its child finally lands in the runtime map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnOutcome {
    /// No Stop arrived while the run sequence was working — carry on.
    Proceed,
    /// A Stop was claimed inside the pre-registration window. The card already says `stopping` and
    /// the Stop sequence found nothing to signal, so this brand-new tree is *only* reachable from
    /// here: kill it, or it runs unowned for the rest of the app's lifetime.
    CancelRun,
}

/// What a Stop can actually act on, decided under the runtime lock so it cannot race the spawn.
#[derive(Debug)]
pub enum StopClaim {
    /// A kill primitive is registered (or demonstrably never existed) — signal it and verify.
    Kill(KillTarget),
    /// A Run is inside its pre-registration window. Wait for it to settle, then re-claim; the
    /// receiver resolves when the run sequence has cancelled itself (or simply returned).
    AwaitSpawn(watch::Receiver<bool>),
}

impl ProjectRuntime {
    /// Run's bookkeeping, applied under the same lock as the §6 `Run` transition.
    pub fn begin_run(&mut self, spawn_in_flight: watch::Receiver<bool>) {
        self.user_stop = false;
        // §8 buffer lifecycle: cleared at the start of each Run.
        self.logs.clear();
        // The previous run's kill primitive is retired here — one of the only two places it may be
        // (the other is a *verified* stop).
        self.kill_pid = None;
        self.child_registered = false;
        self.exited = None;
        #[cfg(windows)]
        {
            self.job = None;
        }
        self.spawn_in_flight = Some(spawn_in_flight);
    }

    /// True if a Stop has already been claimed for this run. Checked once more immediately before
    /// the spawn, so a Stop that landed during the environment resolution costs no process at all.
    ///
    /// The status test is a belt-and-braces assertion of the same thing from the §6 side: while a
    /// run sequence is working, its project can only legally be in one of the phase statuses.
    pub fn run_cancelled(&self) -> bool {
        self.user_stop
            || !matches!(
                self.status,
                Status::Updating | Status::Installing | Status::Starting
            )
    }

    /// Publishes the freshly spawned child as this project's kill target — and answers, under the
    /// same lock, whether a Stop beat it to the punch.
    pub fn register_child(
        &mut self,
        pid: Option<u32>,
        exited: watch::Receiver<bool>,
        path_searched: String,
    ) -> SpawnOutcome {
        self.kill_pid = pid;
        self.child_registered = true;
        self.exited = Some(exited);
        self.path_searched = Some(path_searched);

        if self.run_cancelled() {
            // `spawn_in_flight` stays set: the Stop parked on it must not wake until this run has
            // killed what it just created and the §8 verification has announced the outcome.
            SpawnOutcome::CancelRun
        } else {
            self.spawn_in_flight = None;
            SpawnOutcome::Proceed
        }
    }

    /// The exit watcher's runtime-lock block. It reports the user-stop flag (SPEC.md §6: an exit
    /// without it is a crash) and deliberately touches **nothing else**.
    ///
    /// Retiring the kill primitive here would be the obvious-looking thing to do and is wrong:
    /// reaping the direct child is not the same event as the tree being dead — `npm` exits long
    /// before the grandchildren it started — so a Stop that ends in `stop-failed` would leave the
    /// retry with nothing to signal. See [`Self::kill_pid`].
    pub fn observe_child_exit(&mut self) -> bool {
        self.user_stop
    }

    /// Stop's bookkeeping, applied under the same lock as the §6 `Stop` transition. The flag is set
    /// **before** the kill begins (SPEC.md §8), which is also what the spawn side reads.
    pub fn claim_stop(&mut self) -> StopClaim {
        self.user_stop = true;
        match &self.spawn_in_flight {
            Some(rx) => StopClaim::AwaitSpawn(rx.clone()),
            None => StopClaim::Kill(self.take_kill_target()),
        }
    }

    /// Plan 007: the ready-timeout's kill claim. Sets the same `user_stop` flag `claim_stop` sets
    /// (SPEC.md §6's "someone else owns this exit's announcement") AND hands back the kill target,
    /// so a timeout kill cannot be started without holding the exit watcher's announcement. Making
    /// it one call is deliberate: the two halves were separable, and a caller that took the target
    /// without claiming ownership silently lost the §9 step 7 toast to the watcher's generic
    /// message — that is exactly the bug this plan exists to fix, and it must not be reintroducible
    /// by a future edit that calls `take_kill_target` directly from the timeout path.
    pub fn claim_timeout_kill(&mut self) -> KillTarget {
        self.user_stop = true;
        self.take_kill_target()
    }

    /// Everything the kill needs, taken **out** of the runtime map so the async mutex is never held
    /// across the (up to 5 s) kill (SPEC.md §4). `kill_pid` is copied rather than taken: it is the
    /// retry's only handle on the tree and is cleared solely by [`Self::clear_kill_target`].
    pub fn take_kill_target(&mut self) -> KillTarget {
        KillTarget {
            pid: self.kill_pid,
            had_child: self.child_registered,
            exited: self.exited.clone(),
            // DEVIATION worth naming: the job *is* moved out, because `win32job::Job` owns a handle
            // and is not clonable, and the lock cannot be held across the kill. A failed
            // verification puts it straight back (see `restore_kill_target`) so the retry still owns
            // the primitive.
            #[cfg(windows)]
            job: self.job.take(),
        }
    }

    /// A stop that §8 verification **confirmed**. The only place other than a new Run where the
    /// kill primitive is allowed to disappear.
    pub fn clear_kill_target(&mut self) {
        self.kill_pid = None;
        self.child_registered = false;
        self.spawn_in_flight = None;
        #[cfg(windows)]
        {
            self.job = None;
        }
    }

    /// Verification failed → `stop-failed`, and SPEC.md §6 promises the Stop button retries the
    /// kill. That is only true if the retry still owns a primitive, so the job goes back.
    pub fn restore_kill_target(&mut self, outcome: &mut KillOutcome) {
        self.spawn_in_flight = None;
        #[cfg(windows)]
        {
            if self.job.is_none() {
                self.job = outcome.job.take();
            }
        }
        let _ = outcome;
    }
}

pub type RuntimeMap = HashMap<String, ProjectRuntime>;

// ---------------------------------------------------------------------------------------------
// Event payloads (SPEC.md §7 — FROZEN)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusChangedPayload {
    pub project_id: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLinesPayload {
    pub project_id: String,
    pub lines: Vec<LogLine>,
}

pub const STATUS_CHANGED_EVENT: &str = "status-changed";
pub const LOG_LINES_EVENT: &str = "log-lines";

// ---------------------------------------------------------------------------------------------
// The ONE spawn helper
// ---------------------------------------------------------------------------------------------

/// Which shell wrapper the free-text command goes through.
#[derive(Debug, Clone, Default)]
pub enum ShellKind {
    /// SPEC.md §8: Unix `/bin/sh -c <command>`, Windows `cmd /C <command>`.
    #[default]
    Default,
    /// Unix only — `<login shell> -ilc <command>`, used solely by §8 environment resolution.
    /// On Windows it falls back to `cmd /C` (there is no login shell to source).
    LoginInteractive(String),
}

/// Parameters for [`spawn`]. A struct, not a positional list, because plans 003/004/006 will call
/// this for git, installers, the editor and port-owner lookups.
#[derive(Debug, Clone, Default)]
pub struct SpawnSpec {
    /// Free text, byte-for-byte what the user typed (SPEC.md §5).
    pub command: String,
    pub cwd: Option<PathBuf>,
    /// The cached §8 dev environment, applied as an overlay on the inherited environment.
    pub env: EnvMap,
    /// Per-call additions applied last (e.g. §9.2's non-interactive git variables).
    pub extra_env: Vec<(String, String)>,
    /// True when Hangar must be able to tree-kill this child later (dev command, git, installer).
    /// Unix: gives it its own process group; Windows: puts it in a Job Object. Read-only one-shot
    /// lookups (`lsof`, `tasklist`) pass false.
    pub long_lived: bool,
    /// tokio's own drop-reaper. Only for short helper children whose future may be dropped on a
    /// timeout; it is NOT the §8 kill path (plan 003) and must stay false for project children.
    pub kill_on_drop: bool,
    pub shell: ShellKind,
}

/// A freshly spawned child plus the platform handle needed to kill its whole tree later.
pub struct Spawned {
    pub child: Child,
    /// `None` if the Job Object could not be created or the child could not be assigned to it —
    /// SPEC.md §8's `taskkill` fallback condition.
    #[cfg(windows)]
    pub job: Option<win32job::Job>,
}

/// The single place in this codebase where a `Command` is constructed (SPEC.md §8).
///
/// Known limitation, kept deliberately: children that create their own session (`setsid`/daemonize
/// — Nx daemon, Turborepo daemon, watchman) escape the process group by design and are not Hangar's
/// to kill.
pub fn spawn(spec: &SpawnSpec) -> Result<Spawned, String> {
    let mut cmd = build_command(spec);

    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    // Overlay, not `env_clear`: the login-shell environment is complete for tooling, but clearing
    // would also drop anything the OS gave Hangar that a child legitimately needs.
    cmd.envs(spec.env.iter());
    for (key, value) in &spec.extra_env {
        cmd.env(key, value);
    }

    // SPEC.md §8: stdin is null on EVERY child, so interactive prompts (npx "Ok to proceed?",
    // husky hooks, credential prompts) fail fast instead of hanging forever.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.kill_on_drop(spec.kill_on_drop);

    #[cfg(unix)]
    {
        if spec.long_lived {
            // The whole tree shares one process group, which is what plan 003 signals.
            cmd.process_group(0);
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("could not start the command: {e}"))?;
        return Ok(Spawned { child });
    }

    #[cfg(windows)]
    {
        use win32job::{ExtendedLimitInfo, Job};

        cmd.creation_flags(CREATE_NO_WINDOW);

        // SPEC.md §8: create the job FIRST, spawn, then immediately assign. KILL_ON_JOB_CLOSE means
        // that if Hangar dies for any reason the kernel kills the whole job — no orphans even on a
        // Hangar crash. Breakaway is never granted.
        let job = if spec.long_lived {
            let mut info = ExtendedLimitInfo::new();
            info.limit_kill_on_job_close();
            match Job::create_with_limit_info(&info) {
                Ok(job) => Some(job),
                Err(e) => {
                    eprintln!("hangar: could not create a Job Object ({e}); the taskkill fallback will be used");
                    None
                }
            }
        } else {
            None
        };

        let child = cmd
            .spawn()
            .map_err(|e| format!("could not start the command: {e}"))?;

        let job = job.and_then(|job| match child.raw_handle() {
            // `assign_process` takes the raw process HANDLE as an isize (win32job 2.x).
            Some(handle) => match job.assign_process(handle as isize) {
                Ok(()) => Some(job),
                Err(e) => {
                    eprintln!("hangar: could not assign the child to its Job Object ({e}); the taskkill fallback will be used");
                    None
                }
            },
            None => None,
        });

        return Ok(Spawned { child, job });
    }

    // Unreachable: both supported platform families return above.
    #[allow(unreachable_code)]
    Err("unsupported platform".to_string())
}

#[cfg(unix)]
fn build_command(spec: &SpawnSpec) -> Command {
    match &spec.shell {
        ShellKind::Default => {
            let mut cmd = Command::new("/bin/sh");
            cmd.arg("-c").arg(&spec.command);
            cmd
        }
        ShellKind::LoginInteractive(shell) => {
            let mut cmd = Command::new(shell);
            // Interactive AND login: a non-interactive login zsh reads ~/.zprofile but skips
            // ~/.zshrc, where nvm's init lives (SPEC.md §8).
            cmd.arg("-ilc").arg(&spec.command);
            cmd
        }
    }
}

#[cfg(windows)]
fn build_command(spec: &SpawnSpec) -> Command {
    use std::os::windows::process::CommandExt as _;

    let mut cmd = Command::new("cmd");
    // raw_arg, never arg(): normal arg handling applies MSVC-style quoting that cmd.exe does not
    // parse, mangling commands containing quotes, `&`, `^` or `%` (SPEC.md §8). This also is why
    // `npm`/`pnpm`/`yarn`/`code` — `.cmd` batch shims that `Command::new` cannot execute — must go
    // through `cmd /C`. `as_std_mut` is the documented escape hatch: tokio's Command has no
    // `raw_arg` of its own.
    cmd.as_std_mut().raw_arg("/C").raw_arg(&spec.command);
    cmd
}

// ---------------------------------------------------------------------------------------------
// The dual-stack port probe (SPEC.md §8 verification, §9 steps 1 and 5)
// ---------------------------------------------------------------------------------------------

/// True if **either** `127.0.0.1:port` or `[::1]:port` accepts a TCP connection.
///
/// Both stacks are mandatory, not defensive: Node 17+ resolves `localhost` to `::1` first and most
/// dev servers bind IPv6-only as a result, so an IPv4-only probe reports a perfectly healthy server
/// as dead (SPEC.md §12, row "Server bound to IPv6 localhost only").
///
/// Plan 004 reuses this for ready-polling — which is why it is a helper and not inline in the kill
/// path. It performs exactly ONE attempt per stack; the polling loop and its attempt budget belong
/// to plan 004.
pub async fn port_accepts(port: u16) -> bool {
    let (v4, v6) = tokio::join!(
        connect_accepts(SocketAddr::from((Ipv4Addr::LOCALHOST, port))),
        connect_accepts(SocketAddr::from((Ipv6Addr::LOCALHOST, port))),
    );
    v4 || v6
}

async fn connect_accepts(addr: SocketAddr) -> bool {
    matches!(
        tokio::time::timeout(PORT_PROBE_TIMEOUT, tokio::net::TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

// ---------------------------------------------------------------------------------------------
// Who owns a busy port (SPEC.md §9 step 1) — STRICTLY read-only
// ---------------------------------------------------------------------------------------------

/// SPEC.md §9 step 1's budget for the owner lookup. It runs while the user is waiting on a refused
/// Run, so it is a garnish on the error message and must never become the slow part of it.
pub const PORT_OWNER_TIMEOUT: Duration = Duration::from_secs(2);

/// The process holding a port, for the §9 step 1 message. `None` whenever the lookup fails, times
/// out or returns nothing — the caller falls back to the generic wording rather than guessing.
///
/// **Read-only by design.** SPEC.md §9 is explicit that v0 offers no button to kill this process:
/// it is very often the user's own terminal, and Hangar killing processes it did not spawn is
/// exactly the behaviour the §8 guarantee is careful *not* to claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortOwner {
    pub name: String,
    pub pid: u32,
}

impl std::fmt::Display for PortOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (PID {})", self.name, self.pid)
    }
}

/// Runs the platform's port-owner lookup through the ONE spawn helper, so it inherits the resolved
/// dev environment (`lsof` may live outside launchd's minimal PATH) and, on Windows,
/// `CREATE_NO_WINDOW` — a console window flashing over a *failed* Run would be its own bug.
pub async fn port_owner(port: u16, env: &EnvMap) -> Option<PortOwner> {
    #[cfg(unix)]
    // `-nP` stops lsof resolving hosts and port names (slow, and we want the raw number);
    // `-sTCP:LISTEN` narrows it to the listener rather than every connected socket.
    let command = format!("lsof -nP -iTCP:{port} -sTCP:LISTEN");
    #[cfg(windows)]
    let command = format!("netstat -ano | findstr :{port}");

    let stdout = run_lookup(&command, env).await?;

    #[cfg(unix)]
    {
        parse_lsof_owner(&stdout)
    }
    #[cfg(windows)]
    {
        // netstat only knows the PID; a second lookup turns it into a name the user recognises.
        let pid = parse_netstat_pid(&stdout, port)?;
        let tasklist = run_lookup(&format!("tasklist /FI \"PID eq {pid}\""), env).await;
        let name = tasklist
            .as_deref()
            .and_then(|out| parse_tasklist_name(out, pid))
            .unwrap_or_else(|| "an unknown process".to_string());
        Some(PortOwner { name, pid })
    }
}

/// One short, read-only helper child. `kill_on_drop` is tokio's own reaper for the timeout case —
/// it is not the §8 kill path and never touches a project's tree.
async fn run_lookup(command: &str, env: &EnvMap) -> Option<String> {
    let spec = SpawnSpec {
        command: command.to_string(),
        env: env.clone(),
        long_lived: false,
        kill_on_drop: true,
        ..SpawnSpec::default()
    };
    let spawned = spawn(&spec).ok()?;
    let output = tokio::time::timeout(PORT_OWNER_TIMEOUT, spawned.child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Parses `lsof -nP -iTCP:<port> -sTCP:LISTEN`, whose first row is a header:
///
/// ```text
/// COMMAND   PID USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
/// node    45548 anas   23u  IPv6 0x1234567890abcdef      0t0  TCP *:3000 (LISTEN)
/// ```
#[cfg(any(unix, test))]
pub fn parse_lsof_owner(stdout: &str) -> Option<PortOwner> {
    stdout
        .lines()
        // A dual-stack server produces one row per stack; they are the same process, so the first
        // row with a parseable pid is the answer.
        .filter(|line| !line.starts_with("COMMAND"))
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let pid = fields.next()?.parse().ok()?;
            Some(PortOwner {
                name: name.to_string(),
                pid,
            })
        })
}

/// One lsof listener row, deduplicated by pid. Plan 041 (Ports panel) needs the USER column too
/// (`sameUser`), which the §9 toast path never needed — kept separate from [`PortOwner`] rather
/// than widening that struct's frozen shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortListener {
    pub name: String,
    pub pid: u32,
    /// lsof field 3. `None` on the Windows fallback, where no equivalent column exists.
    pub user: Option<String>,
}

/// Plan 041's all-rows counterpart to [`parse_lsof_owner`] above — every DISTINCT pid, not just
/// the first. **Not a refinement of it**: `parse_lsof_owner`'s first-row behaviour is depended on
/// by the §9 toast path and its own tests, so this is a new function beside it, left untouched.
///
/// Why "first row" is not good enough here: a dual-stack server produces two rows for one
/// process, which is the assumption `parse_lsof_owner` is allowed to make — but two *different*
/// processes on `127.0.0.1:P` and `[::1]:P` is also legal, and `port_accepts` is `v4 || v6`, so
/// the panel must be able to say "N processes are listening — Hangar will not guess which one"
/// instead of silently naming whichever came first.
#[cfg(any(unix, test))]
pub fn parse_lsof_all_listeners(stdout: &str) -> Vec<PortListener> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.starts_with("COMMAND") {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(name), Some(pid), Some(user)) =
            (fields.next(), fields.next().and_then(|s| s.parse::<u32>().ok()), fields.next())
        else {
            continue; // malformed row — skipped, not an error
        };
        if !seen.insert(pid) {
            continue; // a dual-stack pair for the same process
        }
        out.push(PortListener {
            name: name.to_string(),
            pid,
            user: Some(user.to_string()),
        });
    }
    out
}

/// Plan 041: every distinct listening pid for `port`, through the same spawn helper and cached
/// dev environment as [`port_owner`]. Windows keeps the single-owner shape `port_owner` already
/// has — `netstat` has no USER column, so `user` is always `None` there, same as the doc comment
/// on [`PortListener::user`] says.
pub async fn port_listeners(port: u16, env: &EnvMap) -> Vec<PortListener> {
    #[cfg(unix)]
    {
        let command = format!("lsof -nP -iTCP:{port} -sTCP:LISTEN");
        match run_lookup(&command, env).await {
            Some(stdout) => parse_lsof_all_listeners(&stdout),
            None => Vec::new(),
        }
    }
    #[cfg(windows)]
    {
        match port_owner(port, env).await {
            Some(owner) => vec![PortListener {
                name: owner.name,
                pid: owner.pid,
                user: None,
            }],
            None => Vec::new(),
        }
    }
}

/// One row of the batched `ps -o pid=,ppid=,lstart=,command=` read [`ps_enrich`] runs. The type is
/// not `cfg`-gated even though only the Unix branch of `ps_enrich` ever populates it, so callers on
/// every platform share one shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PsInfo {
    pub ppid: u32,
    pub lstart: String,
    pub command: String,
}

/// Plan 041 step 3: ONE spawned `ps` for every solo-listener pid a `get_port_status` call found,
/// never one child per pid. Unix only — `ps -o lstart=,command=` has no Windows equivalent, and
/// `PortHolder.command`/`.startedAt`/`.parentExited` are documented Unix-only (SPEC.md §7).
#[cfg(unix)]
pub async fn ps_enrich(pids: &[u32], env: &EnvMap) -> HashMap<u32, PsInfo> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let pid_list = pids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    // `command=` must come last: SPEC.md's Ports panel step 3 — it is the one column that can
    // contain spaces, and there is nothing after it to swallow them into.
    let command = format!("ps -o pid=,ppid=,lstart=,command= -p {pid_list}");
    match run_lookup(&command, env).await {
        Some(stdout) => parse_ps_rows(&stdout),
        None => HashMap::new(),
    }
}

#[cfg(windows)]
pub async fn ps_enrich(_pids: &[u32], _env: &EnvMap) -> HashMap<u32, PsInfo> {
    // No Windows read implemented yet — PortHolder's optional fields simply stay absent there.
    HashMap::new()
}

/// Parses `ps -o pid=,ppid=,lstart=,command=` output, split so it is testable without spawning
/// (plan 041's test plan). `pid`/`ppid`/`lstart`'s five tokens are a fixed count; `command` is
/// read as "the rest of the line" so its own internal spaces survive intact.
#[cfg(any(unix, test))]
fn parse_ps_rows(stdout: &str) -> HashMap<u32, PsInfo> {
    let mut out = HashMap::new();
    for line in stdout.lines() {
        if let Some((pid, ppid, lstart, command)) = parse_ps_line(line) {
            out.insert(pid, PsInfo { ppid, lstart, command });
        }
    }
    out
}

/// `pid ppid weekday month day time year command…` — the first 7 whitespace-separated tokens are
/// fixed (`lstart`'s "Wed Aug  5 13:53:00 2026" is always 5 of them), then whatever remains,
/// trimmed, is the command. A short or unparseable line is skipped, not an error.
#[cfg(any(unix, test))]
fn parse_ps_line(line: &str) -> Option<(u32, u32, String, String)> {
    let mut rest = line;
    let mut tokens: Vec<&str> = Vec::with_capacity(7);
    for _ in 0..7 {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        tokens.push(&rest[..end]);
        rest = &rest[end..];
    }
    let pid = tokens[0].parse().ok()?;
    let ppid = tokens[1].parse().ok()?;
    let lstart = tokens[2..7].join(" ");
    let command = rest.trim_start();
    (!command.is_empty()).then(|| (pid, ppid, lstart, command.to_string()))
}

/// Parses `netstat -ano | findstr :<port>`, where the PID is the last column:
///
/// ```text
///   TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       4321
/// ```
///
/// `findstr :<port>` matches the substring anywhere, so rows for a *remote* port (`…:54321`) or a
/// different local port can come back too. Only a LISTENING row whose local address ends in exactly
/// `:<port>` is ours.
#[cfg(any(windows, test))]
pub fn parse_netstat_pid(stdout: &str, port: u16) -> Option<u32> {
    let suffix = format!(":{port}");
    stdout.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // proto, local, remote, state, pid
        if fields.len() < 5 || !fields[3].eq_ignore_ascii_case("LISTENING") {
            return None;
        }
        if !fields[1].ends_with(&suffix) {
            return None;
        }
        fields[4].parse().ok()
    })
}

/// Parses `tasklist /FI "PID eq <pid>"`:
///
/// ```text
/// Image Name                     PID Session Name        Session#    Mem Usage
/// ========================= ======== ================ =========== ============
/// node.exe                      4321 Console                    1     45,678 K
/// ```
///
/// The image name can contain spaces, so it is read as "everything before the PID column" rather
/// than as the first whitespace-separated field.
#[cfg(any(windows, test))]
pub fn parse_tasklist_name(stdout: &str, pid: u32) -> Option<String> {
    let needle = pid.to_string();
    stdout.lines().find_map(|line| {
        // The PID has to match a whole column, not a substring: a low pid like `1` would otherwise
        // match inside the Session# or Mem Usage columns and truncate the name to nothing.
        let (at, _) = line.match_indices(&needle).find(|(i, _)| {
            let before = line[..*i].chars().next_back();
            let after = line[i + needle.len()..].chars().next();
            before.is_none_or(char::is_whitespace) && after.is_none_or(char::is_whitespace)
        })?;
        let name = line[..at].trim();
        // Skips the header and `====` rules, and the "no tasks are running" line, none of which
        // have an image name in front of the number.
        (!name.is_empty() && !name.starts_with('=')).then(|| name.to_string())
    })
}

// ---------------------------------------------------------------------------------------------
// SPEC.md §9 step 3 — the per-canonical-path mutex ("Two projects on one repo ... is a legitimate
// setup ... and without this they would run `git pull` and `npm install` in the same directory
// simultaneously").
// ---------------------------------------------------------------------------------------------

/// Deliberately a process-wide static rather than a field on `AppState`: the single-instance
/// plugin (SPEC.md §4) already guarantees there is exactly one Hangar process per machine, every
/// `run_project` call goes through this same module, and it keeps this plan's diff to `run.rs` /
/// `process.rs` / `registry.rs` / `Cargo.toml` rather than also touching `commands.rs`'s state
/// struct for one coordination primitive with no UI-visible shape.
static PATH_MUTEXES: LazyLock<StdMutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

/// SPEC.md §9 step 3: one mutex per **canonicalized** path, so `/foo` and `/foo/` (or a symlink)
/// serialize against each other too. Held across both the `updating` and `installing` phases by
/// the caller; re-checking the lockfile hash after acquiring it (not done here — that is the
/// caller's job once it holds the guard) is what lets a second project skip a now-redundant
/// install.
pub async fn lock_project_path(path: &Path) -> tokio::sync::OwnedMutexGuard<()> {
    // Canonicalize on a best-effort basis: a path that fails to canonicalize (already gone, a
    // permissions quirk) still gets a mutex, just keyed on its literal form — worse serialization
    // in that corner case, never a panic or a skipped lock.
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mutex = {
        let mut mutexes = PATH_MUTEXES.lock().unwrap_or_else(|e| e.into_inner());
        mutexes.entry(key).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
    };
    mutex.lock_owned().await
}

// ---------------------------------------------------------------------------------------------
// SPEC.md §9 step 2 — the `updating` phase: is this folder even a git repo?
// ---------------------------------------------------------------------------------------------

/// What `git rev-parse --is-inside-work-tree` told us. SPEC.md §12: only [`GitMissing`] earns a log
/// line ("git not found — skipping update"); [`NotRepo`] skips the pull silently.
///
/// [`GitMissing`]: GitAvailability::GitMissing
/// [`NotRepo`]: GitAvailability::NotRepo
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitAvailability {
    IsRepo,
    NotRepo,
    GitMissing,
}

/// Bounded like [`port_owner`]'s lookups: this is read-only and near-instant, so it is never
/// registered as a kill target (SPEC.md §8 reserves that bookkeeping for long-lived children).
const GIT_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// SPEC.md §9 step 2 / §12: decides whether the pull phase should run at all, before anything is
/// narrated or `updating` is entered — a skipped phase must never flash the status (SPEC.md §12
/// "Not a git repo | Skip pull silently").
pub async fn check_git_repo(path: &Path, env: &EnvMap) -> GitAvailability {
    let spec = SpawnSpec {
        command: "git rev-parse --is-inside-work-tree".to_string(),
        cwd: Some(path.to_path_buf()),
        env: env.clone(),
        long_lived: false,
        kill_on_drop: true,
        ..SpawnSpec::default()
    };
    let Ok(spawned) = spawn(&spec) else {
        return GitAvailability::GitMissing;
    };
    let Ok(Ok(output)) =
        tokio::time::timeout(GIT_CHECK_TIMEOUT, spawned.child.wait_with_output()).await
    else {
        // A wedged or slow check is not evidence either way; treat it like "not a repo" and let the
        // run proceed rather than block Run on a hung read-only lookup.
        return GitAvailability::NotRepo;
    };
    interpret_git_check(output.status.code())
}

/// The exit-code interpretation half of [`check_git_repo`], pulled out so it is unit-testable
/// without spawning a real `git` (or needing one absent from PATH to test the "missing" branch).
fn interpret_git_check(exit_code: Option<i32>) -> GitAvailability {
    match exit_code {
        Some(0) => GitAvailability::IsRepo,
        Some(code) if is_tool_not_found_exit(code) => GitAvailability::GitMissing,
        _ => GitAvailability::NotRepo,
    }
}

// ---------------------------------------------------------------------------------------------
// SPEC.md §9 step 3 — the install decision: which lockfile, its hash, and the four branches
// ---------------------------------------------------------------------------------------------

/// Which installer a lockfile implies. SPEC.md §9 step 3's search order — `package-lock.json`,
/// `pnpm-lock.yaml`, `yarn.lock` — is encoded in [`find_lockfile`], not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockfileKind {
    Npm,
    Pnpm,
    Yarn,
}

impl LockfileKind {
    /// The exact command SPEC.md §9 step 3 names for each manager.
    pub fn install_command(self) -> &'static str {
        match self {
            LockfileKind::Npm => "npm install",
            LockfileKind::Pnpm => "pnpm install",
            LockfileKind::Yarn => "yarn",
        }
    }
}

/// SPEC.md §9 step 3: "hash the lockfile (`package-lock.json` | `pnpm-lock.yaml` | `yarn.lock`,
/// first found)". The order itself is testable on its own ([`find_lockfile`] below does the actual
/// `is_file()` check against it).
pub const LOCKFILE_SEARCH_ORDER: [(&str, LockfileKind); 3] = [
    ("package-lock.json", LockfileKind::Npm),
    ("pnpm-lock.yaml", LockfileKind::Pnpm),
    ("yarn.lock", LockfileKind::Yarn),
];

/// Finds the first lockfile present in `project_dir`, per SPEC.md §9 step 3's fixed search order.
/// `None` means "no lockfile at all" — SPEC.md §9: "skip hashing and installing entirely".
pub fn find_lockfile(project_dir: &Path) -> Option<(LockfileKind, PathBuf)> {
    LOCKFILE_SEARCH_ORDER.iter().find_map(|(name, kind)| {
        let candidate = project_dir.join(name);
        candidate.is_file().then_some((*kind, candidate))
    })
}

/// SPEC.md §9 step 3: "SHA-256" of the lockfile's bytes, as a lowercase hex string — the same shape
/// `Project.last_lockfile_hash` is stored in.
pub fn hash_lockfile(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(format!("{digest:x}"))
}

/// SPEC.md §9 step 3's three-way OR, as a pure function — the whole install decision in one place
/// so all four branches ((a) unset, (b) differs, (c) `node_modules` missing, and "none of the
/// above") are testable without touching a filesystem or spawning anything.
pub fn needs_install(
    last_hash: Option<&str>,
    current_hash: &str,
    node_modules_exists: bool,
) -> bool {
    match last_hash {
        None => true,                         // (a) lastLockfileHash unset
        Some(last) if last != current_hash => true, // (b) hash differs
        _ => !node_modules_exists,            // (c) node_modules missing
    }
}

// ---------------------------------------------------------------------------------------------
// The kill paths (SPEC.md §8 — the acceptance-test level requirement)
// ---------------------------------------------------------------------------------------------

/// Everything the kill needs, taken **out** of the runtime map before the sequence starts so the
/// async mutex is never held across the (up to 5 s) kill (SPEC.md §4).
#[derive(Debug, Default)]
pub struct KillTarget {
    /// The retained kill handle — [`ProjectRuntime::child_pid`]. Unix: the process-group id.
    /// Windows: the direct child's pid, used only by the `taskkill` fallback.
    pub pid: Option<u32>,
    /// True when a child *was* registered for this run and its tree has never been confirmed dead.
    /// With no `pid`/`job` left to signal that is an **unverifiable** state, not a verified death —
    /// see the early returns in [`kill_tree`].
    pub had_child: bool,
    /// The exit watcher's reap signal — see [`ProjectRuntime::exited`].
    pub exited: Option<watch::Receiver<bool>>,
    /// The project's Job Object. Kept alive for the whole sequence: it is both the kill primitive
    /// and the verification source, and dropping it (KILL_ON_JOB_CLOSE) is the final backstop.
    #[cfg(windows)]
    pub job: Option<win32job::Job>,
}

/// The result of one kill attempt. `death_confirmed` is the FIRST half of §8 verification; the port
/// check is the second and is deliberately performed by the caller only after this is true.
#[derive(Debug, Default)]
pub struct KillOutcome {
    pub death_confirmed: bool,
    /// `system` log lines narrating what actually happened (SPEC.md §7: "kill results").
    pub notes: Vec<String>,
    /// Handed back so a failed verification can return the primitive to the runtime map and SPEC.md
    /// §6's "`stop-failed` | Stop clicked | retry the kill" is a real retry. On a confirmed stop the
    /// caller simply drops it, and with KILL_ON_JOB_CLOSE that drop is the final backstop.
    #[cfg(windows)]
    pub job: Option<win32job::Job>,
}

/// SPEC.md §8's death check when there is nothing left to signal. `had_child` is the whole question:
/// a Stop pressed with no child of ours anywhere is trivially a confirmed death, but a run whose
/// primitive went missing is a *failure to verify* — and reporting it as death is exactly how a
/// `stop-failed` retry launders itself into a green "stopped" card while orphans keep running.
fn nothing_to_signal(had_child: bool, mut notes: Vec<String>) -> KillOutcome {
    if had_child {
        notes.push(
            "no process group or job is registered for this project any more, so its death \
             cannot be confirmed"
                .to_string(),
        );
    }
    KillOutcome {
        death_confirmed: !had_child,
        notes,
        #[cfg(windows)]
        job: None,
    }
}

/// Waits for the exit watcher to reap the child, bounded by `budget`. Returns true if it was
/// reaped (or if there is nothing to wait for).
async fn wait_for_exit(exited: &Option<watch::Receiver<bool>>, budget: Duration) -> bool {
    let Some(rx) = exited else {
        return true;
    };
    let mut rx = rx.clone();
    if *rx.borrow() {
        return true;
    }
    tokio::time::timeout(budget, async move {
        let _ = rx.wait_for(|reaped| *reaped).await;
    })
    .await
    .is_ok()
}

#[cfg(unix)]
fn signal_group(pgid: u32, signal: i32) -> std::io::Result<()> {
    let pgid = i32::try_from(pgid).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "process id out of range")
    })?;
    // SAFETY: `kill(2)` with a negative pid addresses the whole process group and touches no Rust
    // memory. A negative return is reported through errno, as usual for libc.
    let rc = unsafe { libc::kill(-pgid, signal) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// SPEC.md §9 step 1 (amended 2026-08-10, plan 042): the ONE signal Hangar may send to a process
/// it did not spawn. Unlike [`signal_group`] just above — which negates `pgid` **on purpose** to
/// reach every member of one of Hangar's OWN process groups, and is correct only for our own trees
/// — this function never negates. It takes a single **positive** pid and signals exactly that one
/// process, nothing else. A reviewer must be able to tell the two apart at a glance: `-pgid` there,
/// a bare positive `pid` here. This is the only path in Hangar permitted to touch a stranger's
/// process, and it is gated by `commands::free_port_gate` before it is ever called.
#[cfg(unix)]
pub fn signal_one_process(pid: u32) -> std::io::Result<()> {
    let pid = i32::try_from(pid).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "process id out of range")
    })?;
    // SAFETY: `kill(2)` with a positive pid signals exactly that one process — no negation, no
    // group, no Rust memory touched. A negative return is reported through errno, as usual.
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// SPEC.md §9 step 1: "On Windows the action is unavailable" until command-line and start-time
/// reads are verified on real hardware — `taskkill /PID <pid> /F` with no start-time guard is a
/// weaker operation than that rule authorises. Kept beside the Unix half so `free_port`'s call
/// site compiles unconditionally on both platforms; gate 8 (Unix only) is enforced by this always
/// returning an error here, never by `#[cfg]`-ing the caller out.
#[cfg(windows)]
pub fn signal_one_process(_pid: u32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "freeing a port is not available on Windows yet",
    ))
}

/// SPEC.md §8 verification, Unix half: `kill(-pgid, 0)` returning `ESRCH` means no member of the
/// group is left. `EPERM` means a member exists that we may not signal — that is still *alive*, so
/// it must not count as death.
#[cfg(unix)]
pub fn group_is_gone(pgid: u32) -> bool {
    match signal_group(pgid, 0) {
        Ok(()) => false,
        Err(e) => e.raw_os_error() == Some(libc::ESRCH),
    }
}

#[cfg(unix)]
async fn confirm_group_death(pgid: u32) -> bool {
    let deadline = tokio::time::Instant::now() + DEATH_CONFIRM_TIMEOUT;
    loop {
        if group_is_gone(pgid) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(DEATH_POLL_INTERVAL).await;
    }
}

/// SPEC.md §8, macOS/Linux: SIGTERM to `-pgid`, up to 5 s racing the child's exit, then SIGKILL to
/// `-pgid`; end by awaiting the same `child.wait()` the exit watcher owns; then confirm death.
///
/// Documented limitation (§8): a child that calls `setsid` (Nx / Turborepo daemons, watchman) leaves
/// the group by design and is not Hangar's to kill.
#[cfg(unix)]
pub async fn kill_tree(target: KillTarget) -> KillOutcome {
    let mut notes = Vec::new();

    let Some(pgid) = target.pid else {
        // Stop pressed with no child registered (e.g. the phase between two children). There is
        // nothing of ours alive; the caller still verifies the port. But see `nothing_to_signal`:
        // that is only true when there demonstrably never was one.
        return nothing_to_signal(target.had_child, notes);
    };

    match signal_group(pgid, libc::SIGTERM) {
        Ok(()) => notes.push(format!("sent SIGTERM to process group {pgid}")),
        Err(e) if e.raw_os_error() == Some(libc::ESRCH) => {
            notes.push(format!("process group {pgid} had already exited"));
        }
        Err(e) => notes.push(format!("could not SIGTERM process group {pgid}: {e}")),
    }

    // §8: "wait up to 5 s racing child.wait()". The watcher owns the wait; this races its signal.
    wait_for_exit(&target.exited, TERM_GRACE).await;

    if !group_is_gone(pgid) {
        match signal_group(pgid, libc::SIGKILL) {
            // Not "after 5 s": §8's wait *races* `child.wait()`, so it ends as soon as the direct
            // child is reaped — which is usually well inside the grace. Saying "after 5 s" here
            // would put a measurement in the log that never happened.
            Ok(()) => notes.push(format!(
                "process group {pgid} did not exit on SIGTERM (waited up to {} s) — sent SIGKILL",
                TERM_GRACE.as_secs()
            )),
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) => {}
            Err(e) => notes.push(format!("could not SIGKILL process group {pgid}: {e}")),
        }
    }

    // §8 reaping: never abandon a Child. The watcher does the actual `wait()`; we must not declare
    // anything until it has.
    if !wait_for_exit(&target.exited, REAP_TIMEOUT).await {
        notes.push(format!(
            "the direct child of group {pgid} was not reaped within {} s",
            REAP_TIMEOUT.as_secs()
        ));
    }

    let death_confirmed = confirm_group_death(pgid).await;
    if !death_confirmed {
        notes.push(format!(
            "processes in group {pgid} were still alive {} s after SIGKILL",
            DEATH_CONFIRM_TIMEOUT.as_secs()
        ));
    }

    KillOutcome {
        death_confirmed,
        notes,
    }
}

/// SPEC.md §8, step 5, as a pure function: the two verification results the Stop sequence has to
/// combine, and the status each pair produces. Death is confirmed FIRST and the port is only
/// *asked* when it was — with processes still alive the port answer is meaningless, and answering
/// "free" from it is the false proxy §8 forbids.
///
/// Returns `true` for `stopped`, `false` for `stop-failed`.
pub fn stop_is_verified(death_confirmed: bool, port_still_answers: bool) -> bool {
    death_confirmed && !port_still_answers
}

/// What [`settle_after_kill`] concludes. Plan 014: a `stop_is_verified` `false` used to mean
/// `stop-failed` unconditionally; this splits that case so a confirmed-empty group whose port is
/// held by someone else can still report `stopped`, naming the holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopVerdict {
    /// §8 satisfied — either the port is free, or death is confirmed and every live listener has
    /// been provably attributed to a process outside the killed tree.
    Stopped { foreign_owner: Option<PortListener> },
    /// Death unconfirmed, or the port is held by something that cannot be ruled out as ours.
    StopFailed,
}

/// SPEC.md §8 step 5, extended (plan 014). **Deviation from §8's letter, documented per CLAUDE.md's
/// rule for exactly this situation:** §8 says any listener on the pinned port after a kill is a
/// failed stop. Its INTENT, stated two paragraphs above in the killing section, is narrower — the
/// port check exists to catch OUR OWN survivors, leaked children that never listen. Once
/// `death_confirmed` is true the killed group/job is provably empty (Unix: `kill(-pgid, 0)` returned
/// `ESRCH`; Windows: the job's process count is 0), so a listener that is demonstrably not that
/// group cannot be one of our survivors — it satisfies §8's intent even though it fails §8's literal
/// text. `listeners` must come from [`port_listeners`] (every distinct pid), never [`port_owner`]
/// (first row only) — a second, unlisted listener must not be waved through as free.
///
/// Two cases get NO benefit of the doubt, per this plan's hard limit that guessing optimistic is
/// exactly the failure §8 exists to prevent: a listener whose pid equals `killed_pid` (SPEC.md §16
/// forbids trusting a bare pid match as identity once a pid may have been recycled, so this is
/// treated as *unverifiable*, not as proof it is still ours, but either way it is not attributable
/// as foreign); and an empty `listeners` when the port is busy (the lookup found nothing parseable —
/// unattributable is not evidence of success).
pub fn settle_after_kill(
    death_confirmed: bool,
    port_still_answers: bool,
    killed_pid: Option<u32>,
    listeners: &[PortListener],
) -> StopVerdict {
    if stop_is_verified(death_confirmed, port_still_answers) {
        return StopVerdict::Stopped { foreign_owner: None };
    }
    if !death_confirmed {
        // Not even asked: with processes still alive, attribution built on the port answer would be
        // the same false proxy §8 forbids for the port check itself.
        return StopVerdict::StopFailed;
    }
    // death_confirmed && port_still_answers: attempt attribution before giving up.
    if listeners.iter().any(|l| Some(l.pid) == killed_pid) {
        return StopVerdict::StopFailed;
    }
    match listeners.first() {
        Some(owner) => StopVerdict::Stopped { foreign_owner: Some(owner.clone()) },
        None => StopVerdict::StopFailed,
    }
}

// `TerminateJobObject` is not exposed by `win32job` 2.x (it has create / assign / query only), and
// SPEC.md §8 names it as *the* Windows kill. Declared here rather than adding a whole `windows`
// crate dependency for one call — `kernel32` is already linked by std.
#[cfg(windows)]
#[allow(non_snake_case)]
#[link(name = "kernel32")]
extern "system" {
    fn TerminateJobObject(job: isize, exit_code: u32) -> i32;
}

/// `taskkill` exit code 128 = "the process was not found", i.e. it is already dead. SPEC.md §8 says
/// to treat that as success.
#[cfg(windows)]
const TASKKILL_NOT_FOUND: i32 = 128;

#[cfg(windows)]
const TASKKILL_TIMEOUT: Duration = Duration::from_secs(10);

/// SPEC.md §8, Windows: `TerminateJobObject` on the project's job kills every descendant atomically,
/// including grandchildren whose intermediate parent already exited — which `taskkill /T`
/// structurally misses because it walks live PPID chains only. `taskkill` is a FALLBACK, used only
/// when job assignment failed at spawn.
#[cfg(windows)]
pub async fn kill_tree(target: KillTarget) -> KillOutcome {
    let mut notes = Vec::new();
    let terminated;

    if let Some(job) = &target.job {
        // SAFETY: `job.handle()` is a live job handle owned by `target` for the whole call.
        let ok = unsafe { TerminateJobObject(job.handle(), 1) } != 0;
        if ok {
            notes.push("terminated the job object — the whole process tree".to_string());
        } else {
            notes.push(format!(
                "TerminateJobObject failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        terminated = ok;
    } else if let Some(pid) = target.pid {
        notes.push(
            "no job object was assigned at spawn — falling back to taskkill, which cannot \
             guarantee a tree kill"
                .to_string(),
        );
        let (ok, mut taskkill_notes) = taskkill_tree(pid).await;
        notes.append(&mut taskkill_notes);
        terminated = ok;
    } else {
        // Nothing to signal — verified death only if there demonstrably never was a child.
        return nothing_to_signal(target.had_child, notes);
    }

    if !wait_for_exit(&target.exited, REAP_TIMEOUT).await {
        notes.push(format!(
            "the direct child was not reaped within {} s",
            REAP_TIMEOUT.as_secs()
        ));
    }

    let death_confirmed = confirm_job_death(&target, terminated).await;
    if !death_confirmed {
        notes.push("processes are still alive after the kill".to_string());
    }

    // The job is handed back rather than dropped here: a failed verification returns it to the
    // runtime map so SPEC.md §6's Stop-from-`stop-failed` retry still owns the kill primitive. On a
    // confirmed stop the caller drops it, and with JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE that drop is
    // still the final backstop for anything the terminate somehow missed.
    KillOutcome {
        death_confirmed,
        notes,
        job: target.job,
    }
}

/// SPEC.md §8 verification, Windows half: the job's active-process count is 0, or (when the job is
/// unavailable) `TerminateJobObject`/`taskkill` reported success.
#[cfg(windows)]
async fn confirm_job_death(target: &KillTarget, terminated: bool) -> bool {
    let Some(job) = &target.job else {
        return terminated;
    };
    let deadline = tokio::time::Instant::now() + DEATH_CONFIRM_TIMEOUT;
    loop {
        match job.query_process_id_list() {
            Ok(pids) if pids.is_empty() => return true,
            Ok(_) => {}
            // The count is unavailable — fall back to §8's stated alternative.
            Err(_) => return terminated,
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(DEATH_POLL_INTERVAL).await;
    }
}

/// The §8 fallback, through the ONE spawn helper (so it inherits `CREATE_NO_WINDOW` and never
/// flashes a console window).
#[cfg(windows)]
async fn taskkill_tree(pid: u32) -> (bool, Vec<String>) {
    let spec = SpawnSpec {
        command: format!("taskkill /PID {pid} /T /F"),
        // A one-shot helper: it needs no group/job of its own, and tokio's drop-reaper is the right
        // cleanup if the timeout below fires.
        long_lived: false,
        kill_on_drop: true,
        ..SpawnSpec::default()
    };

    let spawned = match spawn(&spec) {
        Ok(spawned) => spawned,
        Err(e) => return (false, vec![format!("could not run taskkill: {e}")]),
    };

    match tokio::time::timeout(TASKKILL_TIMEOUT, spawned.child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let code = output.status.code().unwrap_or(-1);
            let ok = output.status.success() || code == TASKKILL_NOT_FOUND;
            (
                ok,
                vec![format!("taskkill /PID {pid} /T /F exited with code {code}")],
            )
        }
        Ok(Err(e)) => (false, vec![format!("taskkill failed: {e}")]),
        Err(_) => (
            false,
            vec![format!(
                "taskkill did not finish within {} s",
                TASKKILL_TIMEOUT.as_secs()
            )],
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// Pure text helpers (unit-tested — the parts most likely to be subtly wrong)
// ---------------------------------------------------------------------------------------------

/// Strip ANSI/VT escape sequences (SPEC.md §8: v0 strips; stderr tinting comes from the `stream`
/// field, not from ANSI). Other C0 control characters are dropped too — a raw BEL or backspace in
/// the panel is noise — except tab, which is real layout.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            if c == '\t' || !c.is_control() {
                out.push(c);
            }
            continue;
        }
        match chars.next() {
            // CSI: parameters until a final byte in 0x40..=0x7E (e.g. "\x1b[32m").
            Some('[') => {
                for c2 in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c2) {
                        break;
                    }
                }
            }
            // OSC: terminated by BEL or ST (ESC \) — e.g. window-title sequences.
            Some(']') => {
                while let Some(c2) = chars.next() {
                    if c2 == '\u{7}' {
                        break;
                    }
                    if c2 == '\u{1b}' {
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                        }
                        break;
                    }
                }
            }
            // Character-set designators take one more byte.
            Some('(') | Some(')') | Some('#') => {
                chars.next();
            }
            // Any other two-character escape: the second byte is already consumed.
            Some(_) | None => {}
        }
    }
    out
}

/// SPEC.md §8: truncate any single line beyond 4 KB with an appended marker. Cuts on a char
/// boundary so the result is always valid UTF-8.
pub fn truncate_line(mut line: String) -> String {
    if line.len() <= MAX_LINE_BYTES {
        return line;
    }
    let mut end = MAX_LINE_BYTES;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    line.truncate(end);
    line.push_str(TRUNCATION_MARKER);
    line
}

/// Strip, then truncate. Applied to every byte that comes off a child's stdout/stderr.
pub fn sanitize_line(raw: &str) -> String {
    truncate_line(strip_ansi(raw))
}

/// Splits a byte stream into lines, treating `\r` as a break equivalent to `\n` (SPEC.md §8) and
/// decoding with lossy UTF-8 so invalid bytes can never fail or panic.
#[derive(Debug, Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
    /// The previous byte was `\r`, so a following `\n` is the same break, not an empty line.
    pending_cr: bool,
}

impl LineSplitter {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for &byte in chunk {
            match byte {
                b'\r' => {
                    // A `\r` break whose content sanitizes to nothing is a cursor-rewrite artifact,
                    // not a line: Vite/webpack/next/npm emit `\x1b[2K\r` (or a bare `\r`) once per
                    // animation frame, and pushing those as empty lines would flood the 100 ms
                    // batcher and evict the real output from the 500-line ring.
                    if let Some(line) = self.take_line() {
                        out.push(line);
                    }
                    self.pending_cr = true;
                }
                b'\n' => {
                    if self.pending_cr && self.buf.is_empty() {
                        // second half of a CRLF — the break was already emitted
                    } else {
                        // A `\n` break DOES emit a genuine blank line: dev servers use them for
                        // spacing and dropping them would reflow the panel.
                        out.push(self.take_line().unwrap_or_default());
                    }
                    self.pending_cr = false;
                }
                _ => {
                    self.buf.push(byte);
                    self.pending_cr = false;
                }
            }
        }
        out
    }

    /// The trailing partial line at EOF (a child that exits without a final newline). `None` when
    /// there is nothing buffered, or when what is buffered is escape sequences only.
    pub fn finish(&mut self) -> Option<String> {
        self.take_line()
    }

    /// Drains the buffer and sanitizes it. `None` when the sanitized result is empty — the caller
    /// decides whether an empty break is meaningful.
    fn take_line(&mut self) -> Option<String> {
        let bytes = std::mem::take(&mut self.buf);
        let line = sanitize_line(&String::from_utf8_lossy(&bytes));
        if line.is_empty() {
            None
        } else {
            Some(line)
        }
    }
}

/// SPEC.md §8: if more than 2000 lines arrive in one 100 ms window, keep the newest and prepend a
/// synthetic `system` line — a crash-looping server must not freeze the frontend via the IPC bridge.
pub fn cap_batch(mut batch: Vec<LogLine>) -> Vec<LogLine> {
    if batch.len() <= MAX_LINES_PER_FLUSH {
        return batch;
    }
    let skipped = batch.len() - MAX_LINES_PER_FLUSH;
    let newest = batch.split_off(skipped);
    let mut out = Vec::with_capacity(MAX_LINES_PER_FLUSH + 1);
    out.push(LogLine::system(format!("… {skipped} lines skipped")));
    out.extend(newest);
    out
}

// ---------------------------------------------------------------------------------------------
// Buffer + event plumbing
// ---------------------------------------------------------------------------------------------

/// Append to the ring buffer FIRST (it is the source of truth), then emit the batched event.
pub async fn append_logs(app: &AppHandle, project_id: &str, lines: Vec<LogLine>) {
    if lines.is_empty() {
        return;
    }
    {
        let state = app.state::<AppState>();
        let mut runtime = state.runtime.lock().await;
        let entry = runtime.entry(project_id.to_string()).or_default();
        for line in &lines {
            entry.logs.push(line.clone());
        }
    }
    let _ = app.emit(
        LOG_LINES_EVENT,
        LogLinesPayload {
            project_id: project_id.to_string(),
            lines,
        },
    );
}

/// Hangar's own narration (SPEC.md §7): exit codes, missing tools, kill results.
pub async fn append_system(app: &AppHandle, project_id: &str, line: impl Into<String>) {
    append_logs(app, project_id, vec![LogLine::system(line)]).await;
}

/// The ONE place `status-changed` is emitted (SPEC.md §7: emitted on every transition).
///
/// It only *emits*. The status itself is written by `run::apply_with`, which is the only place a §6
/// transition is decided — the check and the write have to happen under a single lock, and a
/// separate "set" entry point here is exactly how a double-clicked Run gets to double-spawn.
pub fn emit_status(app: &AppHandle, project_id: &str, status: Status, message: Option<String>) {
    let _ = app.emit(
        STATUS_CHANGED_EVENT,
        StatusChangedPayload {
            project_id: project_id.to_string(),
            status,
            message,
        },
    );
}

// ---------------------------------------------------------------------------------------------
// The live pipeline: two readers -> one batching flusher -> ring buffer + event
// ---------------------------------------------------------------------------------------------

/// The live pipeline's completion handle. `child.wait()` returns as soon as the process is reaped —
/// it does not touch the pipes — so up to one full 100 ms flush window of the child's own output is
/// still in flight at that moment. The exit watcher must [`drain`](LogPipeline::drain) this before
/// it narrates the exit, for two reasons:
///
/// 1. **Ordering**: otherwise `process exited with code 1` and the `crashed` card land *before* the
///    stderr line that explains them.
/// 2. **Cross-run corruption**: an undrained flusher outlives its child, so a Run clicked within
///    ~100 ms of a crash would have the dead run's lines appended to its freshly cleared buffer.
pub struct LogPipeline {
    /// Fires when the flusher's channel closes, which happens once both readers have dropped their
    /// sender AND every queued line has been flushed to the ring buffer.
    done: tokio::sync::oneshot::Receiver<()>,
    /// Aborted if the drain times out, so a wedged pipe can never leak into the next run.
    tasks: Vec<tauri::async_runtime::JoinHandle<()>>,
}

/// A wedged pipe (an escaped grandchild holding the write end open) must not stall the `crashed`
/// transition forever.
pub const LOG_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

impl LogPipeline {
    /// Waits for every reader and the flusher to finish, bounded by [`LOG_DRAIN_TIMEOUT`]. On
    /// timeout the tasks are aborted and a `system` line says so.
    pub async fn drain(self, app: &AppHandle, project_id: &str) {
        let Self { done, tasks } = self;
        if tokio::time::timeout(LOG_DRAIN_TIMEOUT, done).await.is_err() {
            for task in &tasks {
                task.abort();
            }
            append_system(
                app,
                project_id,
                "log output did not finish within 2 s — some final lines may be missing \
                 (a child still holds the pipe open)",
            )
            .await;
        }
    }
}

/// Takes stdout and stderr off the child and starts the reader and flusher tasks.
#[must_use = "the exit watcher must drain the pipeline before narrating the exit"]
pub fn attach_log_pipeline(app: &AppHandle, project_id: &str, child: &mut Child) -> LogPipeline {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<LogLine>();
    let mut tasks = Vec::new();

    if let Some(stdout) = child.stdout.take() {
        tasks.push(spawn_reader(stdout, Stream::Stdout, tx.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        tasks.push(spawn_reader(stderr, Stream::Stderr, tx.clone()));
    }
    drop(tx); // so the flusher ends when both readers are done

    let (done_tx, done) = tokio::sync::oneshot::channel();
    tasks.push(spawn_flusher(
        app.clone(),
        project_id.to_string(),
        rx,
        done_tx,
    ));

    LogPipeline { done, tasks }
}

fn spawn_reader<R>(
    reader: R,
    stream: Stream,
    tx: UnboundedSender<LogLine>,
) -> tauri::async_runtime::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let mut reader = reader;
        let mut splitter = LineSplitter::default();
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    for line in splitter.push(&chunk[..n]) {
                        if tx.send(LogLine { stream, line }).is_err() {
                            return;
                        }
                    }
                }
            }
        }
        if let Some(line) = splitter.finish() {
            let _ = tx.send(LogLine { stream, line });
        }
    })
}

fn spawn_flusher(
    app: AppHandle,
    project_id: String,
    mut rx: UnboundedReceiver<LogLine>,
    done: tokio::sync::oneshot::Sender<()>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        // Batched: at most one `log-lines` event every 100 ms (SPEC.md §8).
        while let Some(first) = rx.recv().await {
            let mut batch = vec![first];
            let deadline = tokio::time::Instant::now() + FLUSH_INTERVAL;
            loop {
                match tokio::time::timeout_at(deadline, rx.recv()).await {
                    Ok(Some(line)) => batch.push(line),
                    // senders gone, or the window closed — flush what we have
                    Ok(None) | Err(_) => break,
                }
            }
            append_logs(&app, &project_id, cap_batch(batch)).await;
        }
        // `recv()` returned None: both readers dropped their sender and everything they produced is
        // now in the ring buffer. Only here is it safe to declare the run over.
        let _ = done.send(());
    })
}

/// Exit codes that mean "the shell could not find the command", i.e. the tool was not on the PATH
/// Hangar handed the child.
///
/// SPEC.md §12, row "Hangar launched from Dock/Finder on macOS with nvm-managed npm": *"if a tool is
/// missing anyway, the log line shows the PATH searched"*, and SPEC.md §8: *"If a tool still resolves
/// to nothing, the error log line must show the PATH that was searched."* This is the realistic
/// failure — `/bin/sh` itself always exists, so `spawn` succeeds and it is the shell that reports
/// `npm: command not found` and exits 127. Plans 003/004 must not remove this.
pub fn is_tool_not_found_exit(code: i32) -> bool {
    matches!(
        code,
        // POSIX sh: 127 = command not found, 126 = found but not executable.
        126 | 127
        // cmd.exe: 9009 = "'npm' is not recognized as an internal or external command".
        | 9009
    )
}

/// One task per running project that awaits `child.wait()` — SPEC.md §8 requires that no `Child`
/// handle is ever abandoned without being waited: this is both the crash trigger and the reaper.
///
/// `pipeline` is the handle from [`attach_log_pipeline`]; it is drained before anything is narrated
/// so the child's own last words always precede `process exited …` and the `crashed` card.
///
/// `exited` is signalled last, once the child is reaped, its output is flushed and its status is
/// settled — that is what the kill sequence awaits instead of a second `wait()` on the same Child.
pub fn spawn_exit_watcher(
    app: AppHandle,
    project_id: String,
    child: Child,
    pipeline: LogPipeline,
    exited: watch::Sender<bool>,
) {
    tauri::async_runtime::spawn(async move {
        let mut child = child;
        let result = child.wait().await;

        // `wait()` only drops stdin and reaps — the readers can still hold unflushed output.
        pipeline.drain(&app, &project_id).await;

        let exit_code = result.as_ref().ok().and_then(|status| status.code());

        let (message, exit_note) = match &result {
            Ok(status) => match status.code() {
                Some(code) => (
                    format!("process exited with code {code}"),
                    format!("exit code {code}"),
                ),
                None => (
                    format!("process exited ({status})"),
                    format!("{status}"),
                ),
            },
            Err(e) => (
                format!("could not wait for the process: {e}"),
                format!("wait failed: {e}"),
            ),
        };

        // The log line lands in the buffer before the status flips, so the panel always explains
        // the card.
        append_system(&app, &project_id, message).await;

        // SPEC.md §8/§12: when the command itself was never found, say what PATH was searched —
        // this is the nvm/fnm/volta-from-Dock failure the whole §8 environment resolution exists to
        // prevent, and the user cannot debug it without seeing the PATH.
        if exit_code.is_some_and(is_tool_not_found_exit) {
            let path_searched = {
                let state = app.state::<AppState>();
                let runtime = state.runtime.lock().await;
                runtime
                    .get(&project_id)
                    .and_then(|entry| entry.path_searched.clone())
            };
            if let Some(path) = path_searched {
                append_system(&app, &project_id, format!("PATH searched: {path}")).await;
            }
        }

        // SPEC.md §6: a child exit with the user-stop flag NOT set is a crash. See
        // `ProjectRuntime::observe_child_exit` for what this block must NOT do.
        let (user_stop, from) = {
            let state = app.state::<AppState>();
            let mut runtime = state.runtime.lock().await;
            let entry = runtime.entry(project_id.clone()).or_default();
            // The status is read under the SAME lock as the flag: §9 step 5's diagnosis turns on
            // whether the exit interrupted `starting` (never answered on the port) or `running` (an
            // ordinary crash), and a status re-read after the lock could see a later transition.
            (entry.observe_child_exit(), entry.status)
        };

        // With the flag set, the Stop sequence that set it is awaiting this very exit and announces
        // the outcome itself — `stopped` only once §8 verification has confirmed the tree is dead,
        // `stop-failed` otherwise. Announcing `stopped` from here would show a settled, green
        // "Stopped" card for up to 3 s while processes are still running, which is precisely the
        // silent lie §8 forbids. Either way the exit never reads as `crashed`, which is the §6
        // guarantee ("a user Stop must never display as `crashed`").
        if !user_stop {
            // SPEC.md §9 step 5: an exit while still `starting` means the command never answered on
            // the port, and that has its own diagnosis — "did you pick a script that starts a
            // server?" for exit 0, the exit code otherwise. `run` owns the §9 wording; this is the
            // one place that knows the exit actually happened, so it asks.
            let message = crate::run::exit_message(&app, &project_id, from, exit_code, &exit_note)
                .await;
            let _ = crate::run::apply(
                &app,
                &project_id,
                crate::run::Trigger::ChildExit { user_stop: false },
                Some(message),
            )
            .await;
        }

        // Last: the child is reaped, its output is flushed and its status is settled.
        let _ = exited.send(true);
    });
}

/// The `updating`/`installing` phase children's own reaper (SPEC.md §9 steps 2-3). Unlike
/// [`spawn_exit_watcher`], it applies no §6 transition — a git-pull or install exit is not
/// automatically a crash (git failures warn-and-continue; install failures get their own `crashed`
/// wording) — `run.rs` decides that once it has the exit code via `done`. What it still does,
/// identically to the dev command's watcher, is reap the child, drain its log pipeline and signal
/// `exited`. Crucially it runs **detached**: a `run_project` future dropped by the §9 step 2 10 s
/// git timeout stops awaiting `done`, but this task keeps running and still reaps the child once
/// the timeout handler's kill lands on it — no zombie, no abandoned `Child` (SPEC.md §8).
pub fn spawn_phase_reaper(
    app: AppHandle,
    project_id: String,
    child: Child,
    pipeline: LogPipeline,
    exited: watch::Sender<bool>,
    done: tokio::sync::oneshot::Sender<Option<i32>>,
) {
    tauri::async_runtime::spawn(async move {
        let mut child = child;
        let result = child.wait().await;
        pipeline.drain(&app, &project_id).await;
        let exit_code = result.ok().and_then(|status| status.code());
        let _ = exited.send(true);
        let _ = done.send(exit_code);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------------------------
    // SPEC.md §9 steps 2-3 — the pure parts: lockfile selection/hashing, the install decision,
    // and the git-check interpretation.
    // -------------------------------------------------------------------------------------------

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hangar-process-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn find_lockfile_prefers_npm_over_pnpm_and_yarn() {
        let dir = scratch_dir("lockfile-order");
        std::fs::write(dir.join("yarn.lock"), "").unwrap();
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();
        assert_eq!(find_lockfile(&dir).map(|(k, _)| k), Some(LockfileKind::Npm));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_lockfile_falls_back_pnpm_then_yarn_then_none() {
        let dir = scratch_dir("lockfile-pnpm");
        std::fs::write(dir.join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(dir.join("yarn.lock"), "").unwrap();
        assert_eq!(find_lockfile(&dir).map(|(k, _)| k), Some(LockfileKind::Pnpm));
        let _ = std::fs::remove_dir_all(&dir);

        let dir = scratch_dir("lockfile-yarn");
        std::fs::write(dir.join("yarn.lock"), "").unwrap();
        assert_eq!(find_lockfile(&dir).map(|(k, _)| k), Some(LockfileKind::Yarn));
        let _ = std::fs::remove_dir_all(&dir);

        let dir = scratch_dir("lockfile-none");
        assert!(find_lockfile(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash_lockfile_is_sha256_hex() {
        let dir = scratch_dir("hash");
        let file = dir.join("package-lock.json");
        std::fs::write(&file, b"hello").unwrap();
        // sha256("hello"), verified against `shasum -a 256` — a well-known test vector.
        assert_eq!(
            hash_lockfile(&file).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn needs_install_covers_all_four_branches() {
        // (a) lastLockfileHash unset.
        assert!(needs_install(None, "abc", true));
        // (b) hash differs from the stored one.
        assert!(needs_install(Some("old"), "new", true));
        // (c) node_modules missing, even though the hash matches.
        assert!(needs_install(Some("abc"), "abc", false));
        // None of the three: skip.
        assert!(!needs_install(Some("abc"), "abc", true));
    }

    #[test]
    fn interpret_git_check_covers_repo_missing_and_not_repo() {
        assert_eq!(interpret_git_check(Some(0)), GitAvailability::IsRepo);
        // `git rev-parse` outside a work tree exits 128.
        assert_eq!(interpret_git_check(Some(128)), GitAvailability::NotRepo);
        assert_eq!(interpret_git_check(Some(127)), GitAvailability::GitMissing);
        assert_eq!(interpret_git_check(Some(126)), GitAvailability::GitMissing);
        assert_eq!(interpret_git_check(None), GitAvailability::NotRepo);
    }

    #[test]
    fn lock_project_path_serializes_two_concurrent_holders() {
        // SPEC.md §9 step 3: "two projects sharing a folder cannot pull or install concurrently".
        let dir = scratch_dir("mutex-serialize");
        let cleanup_dir = dir.clone();
        // `block_on` uses Tauri's runtime — SPEC.md §4 forbids creating one of our own.
        tauri::async_runtime::block_on(async move {
            let order: Arc<StdMutex<Vec<&'static str>>> = Arc::new(StdMutex::new(Vec::new()));

            let dir_a = dir.clone();
            let order_a = order.clone();
            let first = tauri::async_runtime::spawn(async move {
                let _guard = lock_project_path(&dir_a).await;
                order_a.lock().unwrap().push("a-acquired");
                tokio::time::sleep(Duration::from_millis(80)).await;
                order_a.lock().unwrap().push("a-released");
            });

            // Head start so the first task is demonstrably holding the guard before the second asks.
            tokio::time::sleep(Duration::from_millis(20)).await;

            let order_b = order.clone();
            let second = tauri::async_runtime::spawn(async move {
                let _guard = lock_project_path(&dir).await;
                order_b.lock().unwrap().push("b-acquired");
            });

            first.await.unwrap();
            second.await.unwrap();

            // "b-acquired" must never appear before "a-released": the second holder could not
            // acquire the mutex until the first one's guard — held across its sleep — was dropped.
            let recorded = order.lock().unwrap().clone();
            assert_eq!(recorded, vec!["a-acquired", "a-released", "b-acquired"]);
        });
        let _ = std::fs::remove_dir_all(&cleanup_dir);
    }

    #[test]
    fn strips_color_sequences() {
        assert_eq!(strip_ansi("\x1b[32mready in 300 ms\x1b[0m"), "ready in 300 ms");
        assert_eq!(strip_ansi("\x1b[1;31mERR\x1b[39;49m!"), "ERR!");
        // A cursor-move sequence and an OSC title both disappear entirely.
        assert_eq!(strip_ansi("a\x1b[2Kb"), "ab");
        assert_eq!(strip_ansi("\x1b]0;my title\x07done"), "done");
        assert_eq!(strip_ansi("plain text"), "plain text");
    }

    #[test]
    fn keeps_tabs_but_drops_other_control_characters() {
        assert_eq!(strip_ansi("a\tb\u{7}c"), "a\tbc");
    }

    #[test]
    fn splits_on_carriage_returns_and_newlines() {
        let mut splitter = LineSplitter::default();
        let lines = splitter.push(b"one\ntwo\rthree\r\nfour");
        assert_eq!(lines, vec!["one", "two", "three"]);
        assert_eq!(splitter.finish(), Some("four".to_string()));
        assert_eq!(splitter.finish(), None);
    }

    #[test]
    fn a_progress_bar_rewriting_with_bare_cr_yields_one_line_per_update() {
        let mut splitter = LineSplitter::default();
        let lines = splitter.push(b"10%\r20%\r30%\r");
        assert_eq!(lines, vec!["10%", "20%", "30%"]);
    }

    #[test]
    fn an_ansi_cursor_rewrite_before_a_cr_does_not_emit_a_blank_line() {
        // Vite/webpack/next/npm redraw their status with `\x1b[2K\r` once per animation frame.
        // Splitting on the raw `\r` before stripping used to turn each frame into an empty line,
        // flooding the batcher and evicting real output from the 500-line ring.
        let mut splitter = LineSplitter::default();
        assert_eq!(splitter.push(b"\x1b[2K\rVITE ready\n"), vec!["VITE ready"]);

        let mut splitter = LineSplitter::default();
        assert_eq!(
            splitter.push(b"\x1b[2K\rbuilding\x1b[2K\rdone\n"),
            vec!["building", "done"]
        );
    }

    #[test]
    fn genuine_blank_lines_from_newlines_are_preserved() {
        let mut splitter = LineSplitter::default();
        assert_eq!(splitter.push(b"one\n\ntwo\n"), vec!["one", "", "two"]);
    }

    #[test]
    fn lines_are_reassembled_across_chunk_boundaries() {
        let mut splitter = LineSplitter::default();
        assert!(splitter.push(b"partial").is_empty());
        assert_eq!(splitter.push(b" line\n"), vec!["partial line"]);
    }

    #[test]
    fn invalid_utf8_does_not_panic_and_is_replaced() {
        let mut splitter = LineSplitter::default();
        let lines = splitter.push(&[0xff, 0xfe, b'o', b'k', b'\n']);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with("ok"), "got {:?}", lines[0]);
        assert!(lines[0].contains('\u{fffd}'), "got {:?}", lines[0]);
    }

    #[test]
    fn a_multi_byte_char_split_across_chunks_survives() {
        let mut splitter = LineSplitter::default();
        let bytes = "café\n".as_bytes();
        assert!(splitter.push(&bytes[..4]).is_empty()); // first byte of é only
        assert_eq!(splitter.push(&bytes[4..]), vec!["café"]);
    }

    #[test]
    fn truncates_lines_over_four_kilobytes() {
        let long = "x".repeat(MAX_LINE_BYTES + 500);
        let out = truncate_line(long);
        assert_eq!(out.len(), MAX_LINE_BYTES + TRUNCATION_MARKER.len());
        assert!(out.ends_with(TRUNCATION_MARKER));

        let short = "x".repeat(MAX_LINE_BYTES);
        assert_eq!(truncate_line(short.clone()), short, "4 KB exactly is not truncated");
    }

    #[test]
    fn truncation_cuts_on_a_char_boundary() {
        // 'é' is two bytes, so the 4096-byte cut lands mid-character.
        let long = "é".repeat(MAX_LINE_BYTES);
        let out = truncate_line(long);
        assert!(out.ends_with(TRUNCATION_MARKER));
        assert!(out.is_char_boundary(out.len() - TRUNCATION_MARKER.len()));
    }

    #[test]
    fn ring_buffer_keeps_the_last_500_lines() {
        let mut buffer = LogBuffer::default();
        for i in 0..RING_CAPACITY {
            buffer.push(LogLine::system(format!("line {i}")));
        }
        assert_eq!(buffer.snapshot().len(), RING_CAPACITY);

        buffer.push(LogLine::system("line 500"));
        assert_eq!(
            buffer.snapshot().len(),
            RING_CAPACITY,
            "the 501st push must evict, not grow"
        );

        let snapshot = buffer.snapshot();
        assert_eq!(snapshot[0].line, "line 1", "the oldest line was evicted");
        assert_eq!(snapshot[RING_CAPACITY - 1].line, "line 500");

        buffer.clear();
        assert!(buffer.snapshot().is_empty());
    }

    #[test]
    fn a_flood_keeps_the_newest_lines_and_reports_the_skip() {
        let batch: Vec<LogLine> = (0..MAX_LINES_PER_FLUSH + 3)
            .map(|i| LogLine::system(format!("line {i}")))
            .collect();
        let capped = cap_batch(batch);

        assert_eq!(capped.len(), MAX_LINES_PER_FLUSH + 1);
        assert_eq!(capped[0].stream, Stream::System);
        assert_eq!(capped[0].line, "… 3 lines skipped");
        assert_eq!(capped[1].line, "line 3", "the newest lines are the ones kept");
        assert_eq!(capped[MAX_LINES_PER_FLUSH].line, format!("line {}", MAX_LINES_PER_FLUSH + 2));
    }

    #[test]
    fn a_batch_at_the_limit_is_untouched() {
        let batch: Vec<LogLine> = (0..MAX_LINES_PER_FLUSH)
            .map(|i| LogLine::system(format!("line {i}")))
            .collect();
        assert_eq!(cap_batch(batch).len(), MAX_LINES_PER_FLUSH);
    }

    #[test]
    fn log_line_serializes_to_the_frozen_wire_shape() {
        let json = serde_json::to_string(&LogLine {
            stream: Stream::Stderr,
            line: "boom".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"stream":"stderr","line":"boom"}"#);
    }

    #[test]
    fn command_not_found_exit_codes_trigger_the_path_searched_line() {
        // SPEC.md §12: "Hangar launched from Dock/Finder on macOS with nvm-managed npm | ... if a
        // tool is missing anyway, the log line shows the PATH searched". `/bin/sh` always exists, so
        // this — not a spawn error — is how a missing npm actually surfaces.
        assert!(is_tool_not_found_exit(127), "sh: command not found");
        assert!(is_tool_not_found_exit(126), "sh: found but not executable");
        assert!(is_tool_not_found_exit(9009), "cmd.exe: not recognized");

        // A dev server that really ran and really failed must NOT get a PATH lecture.
        assert!(!is_tool_not_found_exit(0));
        assert!(!is_tool_not_found_exit(1));
        assert!(!is_tool_not_found_exit(2));
        assert!(!is_tool_not_found_exit(130), "SIGINT via the shell");
    }

    // -----------------------------------------------------------------------------------------
    // SPEC.md §9 step 1 — parsing the read-only port-owner lookups
    // -----------------------------------------------------------------------------------------

    #[test]
    fn reads_the_owning_process_out_of_lsof() {
        let stdout = "\
COMMAND   PID USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
node    45548 anas   23u  IPv6 0x1234567890abcdef      0t0  TCP *:3000 (LISTEN)
node    45548 anas   24u  IPv4 0xfedcba0987654321      0t0  TCP *:3000 (LISTEN)
";
        assert_eq!(
            parse_lsof_owner(stdout),
            Some(PortOwner {
                name: "node".into(),
                pid: 45548
            })
        );
        // The §9 wording the toast is built from.
        assert_eq!(
            parse_lsof_owner(stdout).unwrap().to_string(),
            "node (PID 45548)"
        );
    }

    #[test]
    fn an_empty_or_header_only_lsof_yields_no_owner() {
        // Nothing listening, or lsof refused: the caller must fall back to the generic message
        // rather than invent an owner.
        assert_eq!(parse_lsof_owner(""), None);
        assert_eq!(
            parse_lsof_owner("COMMAND   PID USER   FD   TYPE DEVICE SIZE/OFF NODE NAME\n"),
            None
        );
    }

    // -----------------------------------------------------------------------------------------
    // Plan 041 (Ports panel) — the all-rows lsof parser
    // -----------------------------------------------------------------------------------------

    #[test]
    fn a_dual_stack_pair_for_one_process_collapses_to_one_listener() {
        let stdout = "\
COMMAND   PID USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
node    45548 anas   23u  IPv6 0x1234567890abcdef      0t0  TCP *:3000 (LISTEN)
node    45548 anas   24u  IPv4 0xfedcba0987654321      0t0  TCP *:3000 (LISTEN)
";
        assert_eq!(
            parse_lsof_all_listeners(stdout),
            vec![PortListener {
                name: "node".into(),
                pid: 45548,
                user: Some("anas".into()),
            }]
        );
    }

    #[test]
    fn two_distinct_processes_on_the_two_stacks_both_come_back() {
        let stdout = "\
COMMAND   PID   USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
node    45548   anas   23u  IPv6 0x1234567890abcdef      0t0  TCP *:3000 (LISTEN)
python   9001   root   12u  IPv4 0xfedcba0987654321      0t0  TCP *:3000 (LISTEN)
";
        let listeners = parse_lsof_all_listeners(stdout);
        assert_eq!(listeners.len(), 2, "got {listeners:?}");
        assert!(listeners.contains(&PortListener {
            name: "node".into(),
            pid: 45548,
            user: Some("anas".into()),
        }));
        assert!(listeners.contains(&PortListener {
            name: "python".into(),
            pid: 9001,
            user: Some("root".into()),
        }));
    }

    #[test]
    fn a_header_only_lsof_yields_no_listeners() {
        assert_eq!(parse_lsof_all_listeners(""), Vec::new());
        assert_eq!(
            parse_lsof_all_listeners("COMMAND   PID USER   FD   TYPE DEVICE SIZE/OFF NODE NAME\n"),
            Vec::new()
        );
    }

    #[test]
    fn a_malformed_row_is_skipped_not_an_error() {
        let stdout = "\
COMMAND   PID USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
node    45548 anas   23u  IPv6 0x1234567890abcdef      0t0  TCP *:3000 (LISTEN)
garbage line with no pid at all
node    not-a-pid anas 24u IPv4 0x0 0t0 TCP *:3000 (LISTEN)
";
        assert_eq!(
            parse_lsof_all_listeners(stdout),
            vec![PortListener {
                name: "node".into(),
                pid: 45548,
                user: Some("anas".into()),
            }]
        );
    }

    // -----------------------------------------------------------------------------------------
    // Plan 041 (Ports panel) — the batched `ps` enrichment, tested without spawning
    // -----------------------------------------------------------------------------------------

    #[test]
    fn reads_pid_ppid_lstart_and_a_multi_word_command() {
        let stdout =
            "57140     1 Wed Aug  5 13:53:00 2026 /private/tmp/fix-a11y/node_modules/.bin/vite --host\n";
        let rows = parse_ps_rows(stdout);
        let row = rows.get(&57140).expect("pid 57140 must be present");
        assert_eq!(row.ppid, 1);
        assert_eq!(row.lstart, "Wed Aug 5 13:53:00 2026");
        assert_eq!(row.command, "/private/tmp/fix-a11y/node_modules/.bin/vite --host");
    }

    #[test]
    fn parses_every_row_when_several_pids_were_batched_in_one_call() {
        let stdout = "\
45548  1234 Wed Aug  5 13:53:00 2026 node server.js
 9001     1 Thu Aug  6 09:00:12 2026 python -m http.server
";
        let rows = parse_ps_rows(stdout);
        assert_eq!(rows.len(), 2, "got {rows:?}");
        assert_eq!(rows[&45548].ppid, 1234);
        assert_eq!(rows[&45548].command, "node server.js");
        assert_eq!(rows[&9001].ppid, 1);
        assert_eq!(rows[&9001].command, "python -m http.server");
    }

    #[test]
    fn an_unparseable_ps_line_is_skipped_not_an_error() {
        assert_eq!(parse_ps_rows(""), HashMap::new());
        assert_eq!(parse_ps_rows("not enough columns here\n"), HashMap::new());
    }

    #[test]
    fn reads_the_listening_pid_out_of_netstat() {
        let stdout = "\
  TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       4321
  TCP    [::]:3000              [::]:0                 LISTENING       4321
";
        assert_eq!(parse_netstat_pid(stdout, 3000), Some(4321));
    }

    #[test]
    fn netstat_rows_for_other_ports_and_states_are_ignored() {
        // `findstr :3000` matches the substring anywhere, so an ESTABLISHED connection to a remote
        // :53000, and a listener on :13000, both come back in the same output. Neither owns :3000.
        let stdout = "\
  TCP    127.0.0.1:60123        127.0.0.1:53000        ESTABLISHED     9999
  TCP    0.0.0.0:13000          0.0.0.0:0              LISTENING       8888
  TCP    0.0.0.0:3000           0.0.0.0:0              LISTENING       4321
";
        assert_eq!(parse_netstat_pid(stdout, 3000), Some(4321));
        assert_eq!(parse_netstat_pid("", 3000), None);
    }

    #[test]
    fn reads_the_image_name_out_of_tasklist() {
        let stdout = "\
Image Name                     PID Session Name        Session#    Mem Usage
========================= ======== ================ =========== ============
node.exe                      4321 Console                    1     45,678 K
";
        assert_eq!(parse_tasklist_name(stdout, 4321), Some("node.exe".into()));
    }

    #[test]
    fn a_low_pid_does_not_match_inside_another_column() {
        // pid 1 appears in the Session# column of every row. Matching it as a substring would cut
        // the name off and report an owner with a blank name.
        let stdout = "\
Image Name                     PID Session Name        Session#    Mem Usage
========================= ======== ================ =========== ============
My Dev Server.exe                1 Console                    1     45,678 K
";
        assert_eq!(
            parse_tasklist_name(stdout, 1),
            Some("My Dev Server.exe".into()),
            "the image name may contain spaces and must survive intact"
        );
    }

    #[test]
    fn tasklist_with_no_match_yields_no_name() {
        assert_eq!(
            parse_tasklist_name(
                "INFO: No tasks are running which match the specified criteria.\n",
                4321
            ),
            None
        );
    }

    // -----------------------------------------------------------------------------------------
    // The spawn/Stop race and the retained kill primitive.
    //
    // Both of these are regressions with a name: the runtime entry is the only place the spawn
    // side and the Stop sequence meet, and every operation either performs under the runtime lock
    // is a method on it. Driving those methods in order *is* driving the race.
    // -----------------------------------------------------------------------------------------

    fn started_run() -> (ProjectRuntime, watch::Sender<bool>) {
        let mut entry = ProjectRuntime::default();
        let (claim, in_flight) = watch::channel(false);
        entry.begin_run(in_flight);
        // §6: the `Run` claim moves the card to `starting`, which is what makes Stop legal — long
        // before there is anything to kill.
        entry.status = Status::Starting;
        (entry, claim)
    }

    #[test]
    fn a_stop_claimed_while_the_pid_is_unregistered_is_never_a_verified_death() {
        let (mut entry, _claim) = started_run();

        // Stop lands inside the pre-registration window: the `lastRunAt` write, the login-shell
        // environment resolution, and (from plans 004/006) the whole pull/install phase all live
        // here, so this is minutes wide, not microseconds.
        let claim = entry.claim_stop();
        entry.status = Status::Stopping;
        assert!(entry.user_stop, "the flag is set before any kill (§8)");

        match claim {
            StopClaim::AwaitSpawn(_) => {}
            StopClaim::Kill(target) => {
                // The defect, in one line: a target built here has no pid, `kill_tree` signals
                // nothing, and the port probe finds nothing listening because the server has not
                // started yet — so the card announces `stopped` over a tree that is about to come
                // up and will never be reachable again.
                let outcome = tauri::async_runtime::block_on(kill_tree(target));
                panic!(
                    "a Stop in the pre-registration window must not settle the stop itself \
                     (kill_tree reported death_confirmed = {})",
                    outcome.death_confirmed
                );
            }
        }

        // ...and the pid appears afterwards. The spawn side has to observe the Stop under the very
        // same lock that publishes the pid, or the two can still slip past each other.
        let (_exit_tx, exit_rx) = watch::channel(false);
        assert_eq!(
            entry.register_child(Some(4321), exit_rx, "/usr/bin".to_string()),
            SpawnOutcome::CancelRun,
            "the run must cancel itself instead of returning with a live child"
        );

        // And what it cancels with is a real, signalable target.
        let target = entry.take_kill_target();
        assert_eq!(target.pid, Some(4321));
        assert!(target.had_child);
    }

    #[test]
    fn a_stop_before_the_spawn_bails_the_run_without_creating_a_process() {
        let (mut entry, _claim) = started_run();
        assert!(!entry.run_cancelled(), "nothing has been stopped yet");

        entry.claim_stop();
        entry.status = Status::Stopping;

        assert!(
            entry.run_cancelled(),
            "the last check before `spawn` must see the Stop and cost zero processes"
        );
        // Nothing was ever created, so this — and only this — is an honest confirmed death.
        let outcome = tauri::async_runtime::block_on(kill_tree(entry.take_kill_target()));
        assert!(outcome.death_confirmed);
    }

    #[test]
    fn a_second_stop_after_a_stop_failed_still_signals_and_still_evaluates_death() {
        let (mut entry, _claim) = started_run();
        let (exit_tx, exit_rx) = watch::channel(false);
        entry.register_child(Some(4321), exit_rx, "/usr/bin".to_string());
        entry.status = Status::Running;

        // First Stop: there is a primitive, so it is signalled.
        let StopClaim::Kill(first) = entry.claim_stop() else {
            panic!("a registered child must produce a kill target");
        };
        assert_eq!(first.pid, Some(4321));
        entry.status = Status::Stopping;

        // Mid-sequence the exit watcher reaps the DIRECT child — `npm` exits long before the
        // grandchildren it started do — and runs its own runtime-lock block. That reap must not
        // cost us the primitive.
        let _ = exit_tx.send(true);
        assert!(
            entry.observe_child_exit(),
            "the flag was set before the kill, so this exit is a Stop, not a crash"
        );

        // Verification fails anyway (a group member still alive at 3 s, EPERM, a wedged process):
        // §6 row "`stopping` | kill verification fails | `stop-failed`".
        let mut outcome = KillOutcome {
            death_confirmed: false,
            ..KillOutcome::default()
        };
        entry.restore_kill_target(&mut outcome);
        entry.status = Status::StopFailed;

        // §6 row "`stop-failed` | Stop clicked | `stopping` — retry the kill". A retry that owns
        // nothing is not a retry: it signals nothing, and because the survivors are exactly the
        // children that never listen on the port (esbuild service, file watchers — SPEC.md §8's own
        // examples), the port probe waves it through as `stopped`.
        let StopClaim::Kill(retry) = entry.claim_stop() else {
            panic!("a retry must not wait on a spawn that finished long ago");
        };
        assert_eq!(
            retry.pid,
            Some(4321),
            "the retry Stop must still have a process group to signal"
        );
        assert!(
            retry.had_child,
            "and must still know a child existed, so a missing pid can never read as death"
        );

        // Only a *verified* stop retires it.
        entry.clear_kill_target();
        assert_eq!(entry.kill_pid, None);
        assert!(!entry.child_registered);
    }

    /// Plan 007 (review round): guards the ready-timeout's ownership claim structurally, not just
    /// as documentation. `claim_timeout_kill` must do BOTH halves — set `user_stop` so the exit
    /// watcher holds its announcement, AND hand back a usable kill target — or the §9 step 7 toast
    /// silently loses its race to the watcher's generic message again. A mutation that drops the
    /// `self.user_stop = true;` line from `claim_timeout_kill` must fail this test.
    #[test]
    fn a_timeout_kill_claim_holds_the_exit_watcher() {
        let (mut entry, _claim) = started_run();
        let (_exit_tx, exit_rx) = watch::channel(false);
        entry.register_child(Some(4321), exit_rx, "/usr/bin".to_string());
        entry.status = Status::Starting;

        // Before the claim: an ordinary exit still reads as an ordinary crash — nothing is held.
        assert!(
            !entry.observe_child_exit(),
            "an unclaimed entry must not hold the watcher — that would mislabel every ordinary crash"
        );

        let target = entry.claim_timeout_kill();

        // The kill can actually run: the target is not empty.
        assert_eq!(target.pid, Some(4321), "the timeout kill must have a process group to signal");
        assert!(target.had_child);

        // The watcher is held: this is the whole point of Plan 007. If `claim_timeout_kill` ever
        // regresses to taking the target without setting the flag, this goes back to `false` and
        // the exit watcher announces `crashed` itself, racing out the §9 step 7 toast again.
        assert!(
            entry.observe_child_exit(),
            "claim_timeout_kill must hold the exit watcher's announcement"
        );
    }

    #[test]
    fn a_kill_target_that_lost_its_primitive_is_not_a_confirmed_death() {
        // SPEC.md §8: "never silently pretend it stopped". `had_child` is the difference between
        // "there was nothing of ours to kill" and "we lost track of it".
        let lost = tauri::async_runtime::block_on(kill_tree(KillTarget {
            pid: None,
            had_child: true,
            ..KillTarget::default()
        }));
        assert!(
            !lost.death_confirmed,
            "an unsignalable target must fail verification, not launder itself into `stopped`"
        );
        assert!(!lost.notes.is_empty(), "and it must say so in the log");

        let never_spawned = tauri::async_runtime::block_on(kill_tree(KillTarget::default()));
        assert!(
            never_spawned.death_confirmed,
            "a Stop with no child of ours anywhere is genuinely a confirmed death"
        );
    }

    #[test]
    fn kill_verification_needs_death_first_and_then_the_port() {
        // SPEC.md §8 / plan 003 step 3, the mechanics half of the mapping.
        assert!(stop_is_verified(true, false), "death confirmed + port free");
        assert!(
            !stop_is_verified(true, true),
            "death confirmed but the port still answers"
        );
        assert!(!stop_is_verified(false, false), "death not confirmed");
        assert!(!stop_is_verified(false, true));
    }

    /// SPEC.md §15 test 3 — **the orphan test**, in code, against the real kill path.
    ///
    /// Ignored by default because it starts a real `npm run dev` (needs `npm` on PATH and takes a
    /// few seconds). Run it deliberately:
    ///
    /// ```sh
    /// cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture --test-threads=1
    /// ```
    ///
    /// The fixture is shaped like every failure §8 warns about at once:
    /// - `npm` is an intermediate parent that exits before its grandchild;
    /// - the server spawns a child that **never listens on the port** (an esbuild-service / watcher
    ///   stand-in), so a port-only verification would call this stop a success while it still runs;
    /// - that child **ignores SIGTERM**, so the kill has to escalate to SIGKILL on the group.
    ///
    /// Unix only: the fixture and its measurement shell out to `pgrep`/`lsof`, which are the
    /// SPEC.md §15 measurement and have no Windows equivalent here. This gating is what lets the
    /// test binary compile on Windows at all — the Windows kill path's own coverage is the
    /// `#[cfg(windows)]` unit tests, which this allows to compile and run.
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn the_orphan_test_leaves_no_node_processes_behind() {
        const PORT: u16 = 39117;

        let dir = std::env::temp_dir().join(format!(
            "hangar-orphan-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&dir).expect("create the fixture directory");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"hangar-orphan-test","private":true,"version":"0.0.0","scripts":{"dev":"node server.js"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("server.js"),
            r#"const http = require('http');
const { spawn } = require('child_process');
// A child that never listens on the port AND ignores SIGTERM: port-only verification would miss it
// entirely, and a polite SIGTERM would leave it running.
spawn(process.execPath, ['-e', "process.on('SIGTERM', () => {}); setInterval(() => {}, 1000)"], {
  stdio: 'ignore',
});
http.createServer((_req, res) => res.end('ok')).listen(39117, () => console.log('listening'));
"#,
        )
        .unwrap();

        // `block_on` uses Tauri's runtime — SPEC.md §4 forbids creating one of our own, tests
        // included.
        let fixture = dir.clone();
        tauri::async_runtime::block_on(async move {
            let before = count_node_processes().await;

            let spec = SpawnSpec {
                command: "npm run dev".to_string(),
                cwd: Some(fixture),
                long_lived: true,
                ..SpawnSpec::default()
            };
            let spawned = spawn(&spec).expect("spawn npm run dev");
            let mut child = spawned.child;
            let pid = child.id().expect("the child has a pid");

            // The production contract: one task owns `wait()` (reaping) and signals the kill path.
            let (exit_tx, exit_rx) = watch::channel(false);
            tauri::async_runtime::spawn(async move {
                let _ = child.wait().await;
                let _ = exit_tx.send(true);
            });

            let mut answered = false;
            for _ in 0..60 {
                if port_accepts(PORT).await {
                    answered = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            assert!(answered, "the fixture server never answered on {PORT}");

            let during = count_node_processes().await;
            println!("orphan test: node processes before={before} during={during}");
            println!("orphan test: process group {pid} holds:\n{}", list_group(pid).await);
            assert!(
                during > before,
                "the fixture must actually create node processes"
            );

            let outcome = kill_tree(KillTarget {
                pid: Some(pid),
                had_child: true,
                exited: Some(exit_rx),
                #[cfg(windows)]
                job: spawned.job,
            })
            .await;
            for note in &outcome.notes {
                println!("orphan test: {note}");
            }

            assert!(outcome.death_confirmed, "process death was not confirmed");
            assert!(
                !port_accepts(PORT).await,
                "port {PORT} still answers after the kill"
            );

            let after = count_node_processes().await;
            println!("orphan test: node processes after={after}");
            assert_eq!(
                after, before,
                "the node process count must return to baseline"
            );
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `pgrep -f node | wc -l` — SPEC.md §15's own measurement, run through the one spawn helper.
    /// The count includes the measuring shell itself (its argv contains "node"), which is a constant
    /// offset present in every sample and therefore cancels out of the comparison.
    /// Everything currently in the spawned process group — the tree the kill has to reach.
    #[cfg(unix)]
    async fn list_group(pgid: u32) -> String {
        let spec = SpawnSpec {
            command: format!("pgrep -g {pgid} -l"),
            kill_on_drop: true,
            ..SpawnSpec::default()
        };
        let spawned = spawn(&spec).expect("spawn pgrep");
        let output = spawned.child.wait_with_output().await.expect("run pgrep");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[cfg(unix)]
    async fn count_node_processes() -> usize {
        let spec = SpawnSpec {
            command: "pgrep -f node | wc -l".to_string(),
            kill_on_drop: true,
            ..SpawnSpec::default()
        };
        let spawned = spawn(&spec).expect("spawn pgrep");
        let output = spawned.child.wait_with_output().await.expect("run pgrep");
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    }

    #[test]
    fn status_changed_payload_is_camel_case() {
        let json = serde_json::to_string(&StatusChangedPayload {
            project_id: "abc".into(),
            status: Status::Crashed,
            message: None,
        })
        .unwrap();
        assert_eq!(json, r#"{"projectId":"abc","status":"crashed"}"#);
    }

    #[test]
    fn log_lines_payload_is_camel_case() {
        let json = serde_json::to_string(&LogLinesPayload {
            project_id: "abc".into(),
            lines: vec![
                LogLine {
                    stream: Stream::Stdout,
                    line: "starting".into(),
                },
                LogLine {
                    stream: Stream::Stderr,
                    line: "warning".into(),
                },
            ],
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"projectId":"abc","lines":[{"stream":"stdout","line":"starting"},{"stream":"stderr","line":"warning"}]}"#
        );
    }
}
