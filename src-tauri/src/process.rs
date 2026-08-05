//! SPEC.md §8 — the ONE shared spawn helper, the platform kill paths (Job Objects on Windows),
//! the line-oriented log reader, and the per-project 500-line ring buffers.
//!
//! Plan 002 (M2) implements the spawn side, the log pipeline, the status-transition helper and the
//! exit watcher. The kill paths (`TerminateJobObject`, `SIGTERM`/`SIGKILL` to the process group,
//! death-then-port verification, `stop-failed`) are plan 003 and are deliberately absent here.
//!
//! **Every** child process Hangar will ever spawn goes through [`spawn`] — that is the only way the
//! Windows flags (`raw_arg`, `CREATE_NO_WINDOW`, Job Object assignment) and the universal
//! `stdin: null` cannot be forgotten by a later plan. No `Command` may be constructed anywhere else.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

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
    /// SPEC.md §6: set by Stop (plan 003). The exit watcher reads it to decide `stopped` vs
    /// `crashed`. At M2 nothing sets it, so a child exit is always a crash.
    pub user_stop: bool,
    /// PID of the live child, cleared by the exit watcher. On Unix this is also the process-group
    /// id (the child is spawned with `.process_group(0)`), which is what plan 003 signals.
    pub child_pid: Option<u32>,
    /// The PATH the current child was actually given (`DevEnvironment::effective_path()`), recorded
    /// at spawn so the exit watcher can print it when the shell exits "command not found"
    /// (SPEC.md §8 and the §12 nvm-from-Dock row). See [`is_tool_not_found_exit`].
    pub path_searched: Option<String>,
    /// The Job Object the child was assigned to at spawn. `None` means assignment failed and plan
    /// 003 must use the `taskkill /PID <pid> /T /F` fallback (SPEC.md §8).
    #[cfg(windows)]
    pub job: Option<win32job::Job>,
}

impl Default for ProjectRuntime {
    fn default() -> Self {
        Self {
            status: Status::Stopped,
            logs: LogBuffer::default(),
            user_stop: false,
            child_pid: None,
            path_searched: None,
            #[cfg(windows)]
            job: None,
        }
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
pub async fn set_status(
    app: &AppHandle,
    project_id: &str,
    status: Status,
    message: Option<String>,
) {
    {
        let state = app.state::<AppState>();
        let mut runtime = state.runtime.lock().await;
        runtime.entry(project_id.to_string()).or_default().status = status;
    }
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
pub fn spawn_exit_watcher(
    app: AppHandle,
    project_id: String,
    child: Child,
    pipeline: LogPipeline,
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

        // SPEC.md §6: a child exit with the user-stop flag NOT set is a crash; with it set it is a
        // clean stop (plan 003 is what sets the flag — at M2 it is always false).
        let user_stop = {
            let state = app.state::<AppState>();
            let mut runtime = state.runtime.lock().await;
            let entry = runtime.entry(project_id.clone()).or_default();
            entry.child_pid = None;
            entry.user_stop
        };

        let next = if user_stop {
            Status::Stopped
        } else {
            Status::Crashed
        };
        set_status(&app, &project_id, next, Some(exit_note)).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
