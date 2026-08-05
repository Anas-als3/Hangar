# Plan 003: Kill the whole process tree, verifiably, from every state

> **Executor instructions**: Follow this plan step by step. Run every verification command
> and confirm the expected result before moving to the next step. If anything in the "STOP
> conditions" section occurs, stop and report — do not improvise. When done, update the
> status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `plans/README.md` must show 001 and 002 as DONE, and
> `cargo check --manifest-path src-tauri/Cargo.toml` must exit 0 before you change anything.
> `grep -rn "Command::new" src-tauri/src/ | grep -v process.rs` must return no matches — if
> a second spawn site has appeared, STOP.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: plans/002-m2-spawn-logs.md
- **Category**: bug (prevention) — **this is the highest-priority correctness requirement in the project**
- **Planned at**: commit `e74666e`, 2026-08-05

## Why this matters

SPEC.md calls guaranteed process-tree kill the hardest and most important requirement, and
the app's core promise ("Stop → port is free, zero orphaned node processes") is exactly this
plan. Three specific traps make naive implementations fail:

1. **`taskkill /T` cannot guarantee a tree kill.** It walks *live* parent-child links, so a
   grandchild whose intermediate parent already exited (extremely common: `cmd` → `npm` →
   server, where npm exits early) is invisible to it and survives. Worse, a dead
   intermediate's PID can be reused, so `/T /F` can kill an unrelated process. Job Objects
   are the correct primitive, and they also reap everything if Hangar itself is force-killed.
2. **Port-only verification is a false proxy for death.** Children that never listen on the
   port (esbuild service, file watchers) pass a port check while still running. Process
   death must be confirmed *first*, then the port.
3. **Without a `stopping` state, every successful Stop displays as `crashed`** — the kill
   sequence takes up to 5 seconds, the exit watcher fires during it, and the §6 rule
   "child exits while running → crashed" mislabels a deliberate user action as a crash.

## Current state

Plan 002 produced: the single spawn helper (including Job Object creation and assignment on
Windows), the cached environment, the log pipeline with ring buffers, the exit watcher that
awaits `child.wait()`, the user-stop flag (currently always false), and `run_project` which
spawns and immediately reports `running`.

**Read before writing code**: SPEC.md §8 (killing, verification, reaping, quit
interception), §6 (the full status state machine — this plan implements it), §9 step 1
(the dual-stack port probe you will reuse for verification), §12 (edge case rows for Stop
during phases, kill verification failure, app quit).

## Commands you will need

See the gate table in `plans/README.md`. All five gates apply.

## Scope

**In scope**:
- `src-tauri/src/process.rs` (kill paths, verification, port probe helper)
- `src-tauri/src/run.rs` (state machine enforcement, stop sequence)
- `src-tauri/src/commands.rs` (`stop_project`)
- `src-tauri/src/main.rs` (quit interception — both paths)
- `src/components/ProjectCard.tsx`, `src/store.ts` (Stop wiring, `stopping`/`stop-failed` UI)

**Out of scope**:
- Ready-detection, the ready timeout, opening the browser — plan 004. You will write a
  **dual-stack port probe helper** here because kill verification needs one; plan 004 reuses
  it for polling. Do not add polling loops or timeouts.
- `git pull`, lockfile hashing, `npm install` — plan 006. The state machine you implement
  must already treat `updating` and `installing` as stoppable states, but nothing enters
  them yet.
- Startup recovery of orphans from a previous Hangar crash — explicitly parked in SPEC.md
  §16. Do not build it.

## Git workflow

One commit at the end: `Add M3 process-tree kill, stop states, and quit interception`.

## Steps

### Step 1: Implement the dual-stack port probe helper

A single function attempting TCP connect to **both** `127.0.0.1:port` and `[::1]:port` with
a short per-attempt timeout, returning true if **either** accepts. Node 17+ commonly binds
localhost as IPv6 only, so an IPv4-only probe reports a healthy server as dead.

Plan 004 reuses this for ready-polling — write it as a reusable helper, not inline.

**Verify**: `cargo check` → exit 0.

### Step 2: Implement the platform kill paths

Per SPEC.md §8 (killing):

- **Windows**: `TerminateJobObject` on the project's job handle (kills every descendant
  atomically). Fallback **only if** job assignment failed at spawn: `taskkill /PID <pid> /T /F`
  through the plan 002 spawn helper, treating exit code **128** ("not found") as success.
- **Unix**: `SIGTERM` to `-pgid` (the negative process group), wait up to **5 s** racing
  `child.wait()`, then `SIGKILL` to `-pgid`.
- **Both**: end by awaiting the same `child.wait()` future the exit watcher owns — the
  direct child must be reaped, never abandoned, or zombie `sh`/`cmd` processes accumulate
  for the app's lifetime.

**Verify**: `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc`
→ exit 0. A failure inside `src-tauri/src/` is a real failure — this gate is the only thing
on this machine that checks the Windows kill path at all.

### Step 3: Implement kill verification — death first, then port

Per SPEC.md §8. Order matters and is not negotiable:

1. Confirm **process death**: Unix — `kill(-pgid, 0)` returns `ESRCH`, polled up to 3 s;
   Windows — the job's active-process count is 0 (or `TerminateJobObject` returned success).
2. **Then** confirm the port no longer accepts, using the step 1 dual-stack probe.

If **either** check fails → status `stop-failed`. Never silently report `stopped`. The card
keeps a working Stop button so the user can retry.

**Verify**: `cargo check` → exit 0; `grep -n "stop-failed\|StopFailed" src-tauri/src/run.rs`
→ matches (the state is reachable, not just declared).

### Step 4: Implement the full status state machine (SPEC.md §6)

This step is the heart of the plan. Implement §6's table as the *only* place transitions
happen, and enforce it in the backend — a command arriving in an illegal state returns an
error rather than corrupting state.

Critical rules from §6, each of which is a bug if missed:

- Stop is valid from `updating`, `installing`, `starting`, **and** `running` — it sets the
  **user-stop flag** and status `stopping`, then kills whichever child is currently
  registered for that project.
- A child exit observed while the user-stop flag is set → `stopped`, **never** `crashed`.
  This is what stops every successful Stop from displaying as a crash.
- Kill verification failure → `stop-failed`; Stop from `stop-failed` retries.
- Run is rejected from any status other than `stopped` or `crashed`.
- Remove/Edit while status ∉ {`stopped`, `crashed`} must confirm-and-stop first, waiting
  for verified death before proceeding. Wire the backend guard now; plan 005 builds the
  confirm dialog.

**Verify**: `cargo test` → unit tests for the transition function pass, covering at minimum:
Run rejected from `running`; Stop legal from each of the four active states; exit-with-flag →
`stopped`; exit-without-flag → `crashed`; verification failure → `stop-failed`.

### Step 5: Implement quit interception — both paths

Per SPEC.md §8. Tauri 2 requires **both**, and a naive implementation that handles only the
first silently leaks every process tree when a macOS user presses Cmd+Q:

1. `on_window_event` → `WindowEvent::CloseRequested`: if anything is running,
   `api.prevent_close()` and start the confirm flow.
2. The `tauri::Builder::run(|app, event| ...)` callback → `RunEvent::ExitRequested { api, .. }`:
   if running projects exist and a `cleanup_done: AtomicBool` is false, `api.prevent_exit()`
   and start the same flow.

Confirm flow: **never call a blocking dialog API on the main thread** — use the dialog
plugin's async/callback confirm. On confirm, kill all trees (phase children included), set
`cleanup_done = true`, then `app_handle.exit(0)`, which now passes through.

**Verify**: `cargo check` → exit 0; `grep -n "ExitRequested" src-tauri/src/main.rs` → matches;
`grep -n "CloseRequested" src-tauri/src/main.rs` → matches. Both must be present.

### Step 6: Wire the Stop button and the new states in the UI

Per SPEC.md §11: while `stopping`, the primary button shows a spinner and is disabled.
`stop-failed` uses the crashed color token (`#F87171`) and keeps Stop enabled for retry.
A `crashed` card's primary button is **Run** (retry).

**Verify**: `npx tsc --noEmit` → exit 0; `npm run build` → exit 0.

### Step 7: Run all gates, then perform the orphan test manually, then commit

Run SPEC.md §15 test 3 on this machine before committing:

```
pgrep -f node | wc -l              # baseline, after one prior Run of the project
# Run the project in Hangar, wait for it to start, then click Stop
pgrep -f node | wc -l              # must return to baseline
```

Record the actual before/after numbers in your report. If the count does not return to
baseline, that is a STOP condition, not a rounding error.

## Test plan

Rust unit tests (`cargo test`, no new dependency):
- The §6 transition function: every row of the table, including the illegal-transition
  rejections listed in step 4.
- Kill verification result mapping: death-confirmed + port-free → `stopped`; death-confirmed
  + port-still-answering → `stop-failed`; death-not-confirmed → `stop-failed`.

Integration behavior that unit tests cannot cover (reviewer performs manually on macOS):
- SPEC.md §15 test 3 (the orphan test) — numbers recorded in the executor's report.
- SPEC.md §15 test 8 partially: Stop during `starting` returns the card to `stopped` and
  leaves no `node` processes behind.
- Quit with a project running → confirm dialog appears → accepting kills the tree and exits.

Windows runtime behavior is **not** testable on this machine; it is deferred to a human on
Windows per `plans/README.md`.

## Done criteria

- [ ] All five gates in `plans/README.md` pass
- [ ] `cargo test` passes, including the §6 transition table tests from step 4
- [ ] `grep -n "CloseRequested" src-tauri/src/main.rs` and `grep -n "ExitRequested" src-tauri/src/main.rs` both match
- [ ] Verification confirms process death **before** checking the port (reviewer reads the code path)
- [ ] The orphan test was run on this machine and before/after `pgrep -f node | wc -l` numbers are in the report
- [ ] No ready-polling, timeout, browser-opening, git, or install logic was added
- [ ] `plans/README.md` status row for 003 updated

## STOP conditions

Stop and report back if:

- The orphan test fails — `node` process count does not return to baseline after Stop. Report
  the numbers and what survived (`pgrep -fl node`). Do **not** paper over it by adding a
  broader `pkill`, which would kill the user's unrelated processes.
- `cargo check --target x86_64-pc-windows-msvc` fails inside `src-tauri/src/`.
- Kill verification cannot be made to pass because the port stays occupied by something
  Hangar did not spawn — report rather than widening the kill.
- You conclude the state machine in SPEC.md §6 is internally inconsistent. Quote the rows
  that conflict and stop; do not pick one and proceed silently.
- Implementing this appears to require ready-polling or the run sequence's git/install
  phases. It does not — those are plans 004 and 006.

## Maintenance notes

- Plan 004 replaces `run_project`'s immediate `running` transition with real polling and adds
  the timeout path, which per SPEC.md §9 step 7 **must call this plan's kill sequence before
  setting `crashed`**. That ordering is the fix for the spec's worst orphan bug; a reviewer
  of plan 004 should check it explicitly.
- A reviewer should scrutinize: the death-then-port ordering, that the user-stop flag is set
  before the kill begins (not after), that both quit paths exist, and that `taskkill` is only
  a fallback rather than the primary Windows path.
- Documented limitation to preserve: `setsid`/daemonizing children (Nx, Turborepo, watchman)
  escape the group by design and are outside the guarantee (SPEC.md §8).
