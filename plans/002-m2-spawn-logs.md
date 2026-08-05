# Plan 002: Spawn dev servers through one hardened helper and stream their output live

> **Executor instructions**: Follow this plan step by step. Run every verification command
> and confirm the expected result before moving to the next step. If anything in the "STOP
> conditions" section occurs, stop and report — do not improvise. When done, update the
> status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `git log --oneline` — plan 001's commit must be present, and
> `plans/README.md` must show 001 as DONE. `cargo check --manifest-path src-tauri/Cargo.toml`
> must exit 0 *before* you change anything. If it does not, STOP.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH (this is where the app meets the operating system)
- **Depends on**: plans/001-m1-scaffold.md
- **Category**: bug (prevention) + migration
- **Planned at**: commit `e74666e`, 2026-08-05

## Why this matters

Two failure modes in this plan account for most of the difference between an app that works
on the author's machine and one that works at all:

1. **Environment resolution.** A GUI-launched macOS app inherits launchd's minimal `PATH`,
   not the terminal's, and `sh -lc` never reads `~/.zshrc` — which is exactly where nvm,
   fnm, and volta put node. Without SPEC.md §8's startup environment capture, *every Run
   fails with `npm: command not found`* for the most common macOS Node setup. This is the
   single highest-probability real-world break in the whole project.
2. **The single spawn helper.** Every child — dev server, git, installer, editor, port-owner
   lookup — must go through one function, because that is the only way the Windows flags
   (`raw_arg`, `CREATE_NO_WINDOW`, Job Object assignment) and the universal `stdin: null`
   cannot be forgotten by a later plan. `npm`/`pnpm`/`yarn`/`code` are `.cmd` shims on
   Windows that `Command::new` *cannot* execute at all.

A log pipeline that decodes lossily and batches its events is the difference between a
readable panel and a frozen frontend when a crash-looping server floods the IPC bridge.

## Current state

Plan 001 produced the scaffold: Tauri 2 + React + TS + Vite + Tailwind v4, the ACL
capability file, the §13 module layout, `registry.rs` with atomic storage, and the
read-only command slice. `process.rs` and `env_resolve.rs` exist as doc-comment-only stubs
naming this plan.

**Read before writing code**: SPEC.md §8 (process manager — environment resolution,
spawning, log pipeline), §7 (frozen command/event API), §6 (status state machine), §13
(repository layout). This plan does not repeat their content.

## Commands you will need

See the gate table in `plans/README.md`. All five gates apply.

## Scope

**In scope**:
- `src-tauri/src/env_resolve.rs`, `src-tauri/src/process.rs`, `src-tauri/src/run.rs`
- `src-tauri/src/commands.rs`, `src-tauri/src/main.rs` (state registration only)
- `src-tauri/Cargo.toml` (add `tokio` with the §4 feature list; add `win32job` as a
  `[target.'cfg(windows)'.dependencies]` entry with the one-line justification comment
  SPEC.md §4 requires)
- `src/store.ts`, `src/api.ts`, `src/types.ts`, `src/components/LogPanel.tsx`,
  `src/components/ProjectCard.tsx` (wire Run + Show logs only)

**Out of scope** (do NOT touch, even though they look related):
- **Killing anything.** No `TerminateJobObject`, no `SIGTERM`, no Stop button behavior.
  That is plan 003 and splitting it is how half-kill bugs happen. Creating and *assigning*
  the Job Object at spawn time IS in scope (it is spawn-side); terminating it is not.
- **Port polling, ready detection, opening the browser** — plan 004.
- **`git pull`, lockfile hashing, `npm install`** — plan 006. `run_project` in this plan
  spawns the command directly.
- `registry.rs` storage logic — done and verified in plan 001.

## Git workflow

One commit at the end: `Add M2 spawn helper, environment resolution, and log pipeline`.
Do not push.

## Steps

### Step 1: Implement startup environment resolution (`env_resolve.rs`)

Per SPEC.md §8. Resolve `$SHELL` (fallback `/bin/zsh` on macOS, `/bin/bash` on Linux), run
`<shell> -ilc 'env'` with a **5 second timeout**, parse `KEY=VALUE` lines, cache the result
in managed state at app startup. On timeout or failure: emit a `system` log line and fall
back to the inherited environment — never block startup.

`-ilc` (interactive **and** login) is required, not `-lc`: a non-interactive login zsh reads
`~/.zprofile` but skips `~/.zshrc`, where nvm's init lives. Getting this wrong reproduces
the exact bug this step exists to prevent.

On Windows this is a no-op returning the inherited environment.

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → a unit test for the
`KEY=VALUE` parser passes (multi-line values and `=` inside values handled).

### Step 2: Implement the one spawn helper (`process.rs`)

One function, used by every child process this app will ever spawn. Per SPEC.md §8:

- **Windows**: `Command::new("cmd")` with `.raw_arg("/C")` and `.raw_arg(&command)` —
  never regular args (MSVC quoting mangles `&`, `^`, `%`, quotes). `creation_flags(0x08000000)`
  on every spawn. Create a Job Object (`win32job`) with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`,
  spawn, then immediately assign the child to it; keep the job handle in managed state
  alongside the child. Do not grant breakaway.
- **Unix**: `/bin/sh -c <command>` with the cached environment from step 1, `cwd = path`,
  `.process_group(0)`.
- **Both**: `stdin` **null** (a prompting child must fail fast, never hang), stdout and
  stderr piped.

Take a struct parameter (command, cwd, extra env, whether the child is long-lived) rather
than a growing positional list — plans 004 and 006 will call this for git and installers.

**Verify**: `cargo check --manifest-path src-tauri/Cargo.toml` → exit 0;
`cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc` → exit 0
(this is the only gate that compiles your Windows code — treat a failure in
`src-tauri/src/` as a real failure);
`grep -rn "Command::new" src-tauri/src/ | grep -v process.rs` → **no matches** (the helper
is the only place a Command is constructed).

### Step 3: Implement the log pipeline

Per SPEC.md §8 (log pipeline). All of the following are requirements, not options:

- Decode **lossy UTF-8** (never fail on invalid bytes).
- Treat `\r` as a line break equivalent to `\n`.
- Strip ANSI/VT escape sequences before storing.
- Truncate any single line beyond 4 KB with an appended ` …[truncated]` marker.
- Per-project ring buffer of the last **500** `LogLine`s (`VecDeque`) in managed state,
  appended **before** any event is emitted — the buffer is the source of truth.
- Batch events: flush at most every **100 ms** as one `log-lines` event. If more than 2000
  lines arrive in one window, keep the newest and emit a synthetic `system` line
  `… <n> lines skipped`.

Write ANSI stripping and line splitting as pure functions with `#[cfg(test)]` unit tests —
these are the parts most likely to be subtly wrong and the easiest to test.

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → tests pass covering: ANSI
stripping (including a colored `\x1b[32m` sequence), `\r` splitting, 4 KB truncation,
invalid UTF-8 bytes not panicking, ring buffer capping at 500.

### Step 4: Implement the exit watcher and status events

One task per running project that `await`s `child.wait()`. This is both the crash-detection
trigger and the zombie reaper — SPEC.md §8 requires that no `Child` handle is ever abandoned
without being waited.

Per SPEC.md §6: a child exit while the user-stop flag is **not** set → `crashed`, with a
`system` log line `process exited with code <n>`. Introduce the user-stop flag now (plan 003
sets it); at this milestone it is always false.

Emit `status-changed` on **every** transition, exactly per §7. Payload structs derive
`Serialize` with `#[serde(rename_all = "camelCase")]`.

**Verify**: `cargo check` → exit 0; `grep -rn "status-changed" src-tauri/src/` → emitted
from the transition helper, not scattered ad hoc.

### Step 5: Wire `run_project` (spawn-only at this milestone)

In `run.rs`: enforce the SPEC.md §6 guard — reject unless status is `stopped` or `crashed`,
so a double-clicked Run cannot double-spawn. Clear the project's log buffer, set `lastRunAt`,
transition to `starting`, spawn via the step 2 helper.

**Because plan 004 owns ready-detection, transition to `running` immediately after a
successful spawn at this milestone.** Leave a clearly marked comment naming plan 004 as the
one that replaces this with real port polling. Do not invent a placeholder poll.

Add commands `run_project`, `get_log_buffer`, `clear_log_buffer` — names and signatures
exactly as SPEC.md §7 freezes them. `stop_project` is plan 003; do not add a stub that
"sort of" stops.

**Verify**: `cargo check` → exit 0; `npx tsc --noEmit` → exit 0.

### Step 6: Frontend — global listeners, store, log panel

Register **both** event listeners (`status-changed`, `log-lines`) **once at app startup**
into `src/store.ts` — never inside `LogPanel`. SPEC.md §7 is explicit: lines emitted while
the panel is closed must still reach the buffer, and a listener that mounts with the panel
loses them.

`LogPanel` per SPEC.md §11: slide-over, mono font, autoscroll with pause-on-scroll-up,
stderr tinted, `system` lines muted, Esc closes. On open, call `get_log_buffer` to backfill
— subscribe first, then fetch, then drop fetched lines already received live.

Wire the card's Run button and Show logs. The Stop button may render but must be inert
(plan 003 wires it).

**Verify**: `npx tsc --noEmit` → exit 0; `npm run build` → exit 0;
`grep -rn "listen(" src/components/` → no matches (listeners live in the store).

### Step 7: Run all gates and commit

## Test plan

Rust unit tests (no new dependency; `cargo test`):
- `env_resolve`: `KEY=VALUE` parsing, values containing `=`, blank lines ignored.
- Log pipeline: ANSI stripping, `\r` line splitting, 4 KB truncation, lossy UTF-8 on
  invalid bytes, ring buffer capped at 500 (501st push evicts the oldest).

Manual check the reviewer will perform (macOS): register a real Node project, click Run,
confirm live output appears in the panel; kill the dev server from another terminal and
confirm the card flips to `crashed` with the exit code in the log.

## Done criteria

- [ ] All five gates in `plans/README.md` pass
- [ ] `grep -rn "Command::new" src-tauri/src/ | grep -v process.rs` → no matches
- [ ] `grep -rn "listen(" src/components/` → no matches
- [ ] `grep -rn '"-lc"' src-tauri/src/` → no matches (the shell invocation must be `-ilc`)
- [ ] `cargo test` passes with ≥8 tests covering the cases in the test plan
- [ ] No kill/stop/port/git/install logic was added (out of scope — reviewer checks the diff)
- [ ] `plans/README.md` status row for 002 updated

## STOP conditions

Stop and report back if:

- `cargo check --target x86_64-pc-windows-msvc` fails with errors **inside
  `src-tauri/src/`** that you cannot resolve — the Windows spawn path is a spec-critical
  requirement and must not be left broken or `#[cfg]`-ed away to make the gate pass.
- `win32job` does not expose `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` or an equivalent. Report
  the crate's actual API rather than substituting a different mechanism.
- The Tauri 2 event emit API in SPEC.md §7's snippet does not match the installed version
  (per `CLAUDE.md`: follow the compiler, keep the intent, comment the deviation, report it).
- You find yourself needing to kill a process to make something work. That is plan 003 —
  stop and report instead of writing a second kill path that plan 003 will duplicate.

## Maintenance notes

- Plan 003 replaces the "transition to `running` immediately" line in step 5 only after
  plan 004 lands; 003 depends on the user-stop flag and the job handle in managed state
  existing from this plan.
- A reviewer should scrutinize: that `-ilc` is used (not `-lc`), that no `Command` is
  constructed outside `process.rs`, that the ring buffer is appended before emitting, and
  that `child.wait()` is awaited on every path.
- Known limitation to preserve in comments: children that `setsid`/daemonize escape the
  process group by design (SPEC.md §8).
