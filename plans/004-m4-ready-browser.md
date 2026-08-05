# Plan 004: Detect readiness on the real port and hand off to the browser, hands-free

> **Executor instructions**: Follow this plan step by step. Run every verification command
> and confirm the expected result before moving to the next step. If anything in the "STOP
> conditions" section occurs, stop and report — do not improvise. When done, update the
> status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `plans/README.md` must show 001, 002 and 003 as DONE, and
> `cargo check --manifest-path src-tauri/Cargo.toml` must exit 0 before you change anything.
> A dual-stack port probe helper must already exist in `process.rs` from plan 003 — if it
> does not, STOP.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: plans/003-m3-kill-tree.md
- **Category**: bug (prevention)
- **Planned at**: commit `e74666e`, 2026-08-05

## Why this matters

This plan delivers the app's headline promise — click Run, and the browser opens on a
working server with zero keyboard input. It also closes the spec's worst orphan bug. In the
original draft, a ready-timeout marked the project `crashed` but left the server *running*:
the card then offered Run again, the pre-check found the pinned port free (the server had
auto-bumped to the next port), and a second tree was spawned while the first became a
permanently untracked orphan. SPEC.md §9 step 7 now requires killing the tree **before**
setting `crashed`, and that ordering is the single most important line in this plan.

Two smaller traps: polling only IPv4 reports a healthy IPv6-bound server as dead (Node 17+
commonly binds `::1`), and a wall-clock timeout expires instantly when the machine wakes
from sleep, killing a perfectly healthy server mid-compile.

## Current state

Plans 001–003 produced: the scaffold and storage, the single spawn helper with cached
environment and the log pipeline, the full §6 state machine, the platform kill paths with
death-then-port verification, and a reusable dual-stack port probe helper.

`run_project` currently transitions straight to `running` after a successful spawn, with a
comment naming this plan as the one that replaces it.

**Read before writing code**: SPEC.md §9 (run sequence — the exact order), §6 (state
machine), §8 (killing — you call it, you do not reimplement it), §12 (the edge-case rows for
timeout, IPv6, sleep, and instant-exit).

## Commands you will need

See the gate table in `plans/README.md`. All five gates apply.

## Scope

**In scope**:
- `src-tauri/src/run.rs` (the polling loop, timeout, pre-check, browser hand-off)
- `src-tauri/src/process.rs` (port-owner lookup helper only)
- `src-tauri/src/commands.rs` (`open_in_browser`)
- `src/components/ProjectCard.tsx`, `src/store.ts` (toasts for the new failure messages)

**Out of scope**:
- `git pull`, lockfile hashing, `npm install` — plan 006. The run sequence you build goes
  straight from the guard to spawn; leave a clearly marked seam where steps 2–3 insert.
- The phase strip — plan 006. Statuses transition correctly; the strip renders them later.
- Any change to the kill implementation — call plan 003's kill sequence, do not fork it.
- Add/Edit dialogs — plan 005.

## Git workflow

One commit at the end: `Add M4 ready-detection, timeout handling, and browser hand-off`.

## Steps

### Step 1: Implement the port pre-check with owner lookup (SPEC.md §9 step 1)

Before spawning, probe **both** stacks. If either accepts, do **not** spawn. Then run a
read-only owner lookup with a 2 s timeout, through plan 002's spawn helper:

- macOS/Linux: `lsof -nP -iTCP:<port> -sTCP:LISTEN`
- Windows: `netstat -ano | findstr :<port>` then `tasklist /FI "PID eq <pid>"`

Toast: `Port 3000 is in use by node (PID 4321) — is this project running elsewhere?` If the
lookup fails, times out, or returns nothing, fall back to the generic message. **Strictly
read-only** — v0 offers no button to kill that process (SPEC.md §9).

**Verify**: `cargo check` → exit 0. Manual: start any server on a port, register a project
on it, click Run → the toast names the owning process, and nothing is spawned.

### Step 2: Replace the immediate `running` transition with real polling

Per SPEC.md §9 steps 5–6:

- Poll both stacks every **500 ms**.
- The loop **races the child's exit**. If the child exits while `starting`, stop polling
  immediately and set `crashed` — do not wait out the timeout. Exit code 0 gets the specific
  message from §9 step 5 (`… finished (exit 0) without ever answering on port <port> — did
  you pick a script that starts a server (e.g. dev), not build?`); a nonzero exit gets the
  exit-code message. This turns a 60-second mystery into an instant, accurate diagnosis.
- The timeout budget is counted in **completed poll attempts** (`readyTimeoutSec × 2`), not
  wall-clock elapsed. If the gap between two polls exceeds 5 s, the machine slept — do not
  count the gap against the budget.
- On success: wait **300 ms** grace, then `running`.

**Verify**: `cargo test` → unit tests for the budget accounting pass, including a simulated
sleep gap that must not consume the budget.

### Step 3: Implement the timeout path — kill first, then `crashed`

Per SPEC.md §9 step 7, in this exact order:

1. Call **plan 003's kill sequence** on the spawned tree.
2. Wait for confirmed death.
3. **Then** set `crashed`.

Toast per §9 step 7, including both hints (raise the ready timeout; check whether the server
started on another port and pin it in Edit).

Inverting this order — or skipping the kill — recreates the orphan bug this plan exists to
close. A reviewer will check it explicitly.

**Verify**: `cargo test` → a unit test asserts the timeout path invokes the kill sequence
before the `crashed` transition. Manual: SPEC.md §15 test 7 (below).

### Step 4: Open the browser from Rust

On entering `running`, open `url` (or the computed `http://localhost:<port>` default) via
the **opener plugin, called from Rust**. Per SPEC.md §4, plugin calls made from Rust bypass
the ACL and need no capability entry — do not route this through the webview.

Add the `open_in_browser` command from §7 for the overflow-menu action.

**Verify**: `cargo check` → exit 0. Manual: SPEC.md §15 test 1 (below).

### Step 5: Run all gates, perform the manual acceptance tests, and commit

Run these on this machine and record literal observations in your report:

- **§15 test 1**: register a real Node project → click Run → a browser tab opens on the app
  with zero keyboard input, within 5 s of the server being ready.
- **§15 test 7 (the timeout-orphan test)**: occupy the project's pinned port with another
  process so the framework auto-bumps and the ready-check can never succeed → Run → after
  the timeout the card is `crashed` **and** `pgrep -f node | wc -l` is back to baseline.
  Record the before/after numbers. This is the test that proves the kill-then-crash ordering.
- **§15 test 4 bonus**: point a project's command at a script that exits instantly → Run →
  `crashed` appears immediately, not after 60 s.

## Test plan

Rust unit tests (`cargo test`):
- Timeout budget accounting: N attempts consumed correctly; a >5 s inter-poll gap does not
  consume budget; the budget equals `readyTimeoutSec × 2` attempts.
- The timeout path calls kill before transitioning to `crashed` (assert ordering).
- Child-exit-during-`starting` maps to `crashed` with exit-code-0 and nonzero producing
  different messages.

Manual acceptance tests as listed in step 5, with observed numbers in the report.

## Done criteria

- [ ] All five gates in `plans/README.md` pass
- [ ] `cargo test` passes including the ordering and budget tests
- [ ] Port probes are dual-stack everywhere (`grep -n "::1" src-tauri/src/` matches)
- [ ] §15 tests 1 and 7 were run on this machine, with observed results and `pgrep` numbers in the report
- [ ] No git/install/phase-strip/dialog work was added
- [ ] `plans/README.md` status row for 004 updated

## STOP conditions

Stop and report back if:

- §15 test 7 leaves a surviving process after the timeout — that is the exact bug this plan
  closes, and shipping it defeats the plan's purpose.
- The opener plugin's Rust-side API does not match SPEC.md's description (follow the
  compiler and current docs, comment the deviation, report it).
- Making the ready-check pass appears to require auto-detecting the server's real port from
  log output. SPEC.md §3 explicitly forbids silent port auto-detection; the explicit
  one-click repin variant is parked in §16. Report instead of building either.
- You need `git pull` or `npm install` to make a test pass — those are plan 006.

## Maintenance notes

- Plan 006 inserts the `updating` and `installing` phases *before* step 4's spawn, at the
  seam you leave in `run.rs`. The phases must be stoppable, and a killed install must not
  store the lockfile hash.
- A reviewer should scrutinize: the kill-before-`crashed` ordering, that polling races child
  exit rather than sleeping through it, dual-stack probes, and that the 300 ms grace is not
  silently dropped.
- Deferred deliberately: the HTTP ready-check mode and the one-click port repin, both in
  SPEC.md §16.
