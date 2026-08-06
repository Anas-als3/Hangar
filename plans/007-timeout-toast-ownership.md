# Plan 007: Make the ready-timeout own its crash — the §9 step 7 toast must reach the user

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 91be38f..HEAD -- src-tauri/src/run.rs src-tauri/src/process.rs src/store.ts`
> If any of these changed since this plan was written, compare the "Current
> state" excerpts against the live code before proceeding; on a mismatch,
> treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW-MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `91be38f`, 2026-08-06

## Why this matters

When a dev server never answers on its pinned port, SPEC.md §9 step 7 promises a
specific toast: *"Server didn't answer on port \<port\> within \<readyTimeoutSec\> s,
so it was stopped. If it just needs longer (e.g. a first cold compile), raise Ready
timeout in Edit. Check the log — did it start on another port? Pin it in Edit."*
That is the app's main self-help for the two most common failure modes (§12 rows
"Server ready but on a different port" and slow cold compiles).

Today that toast can never reach the user on the normal path, because of an
ordering race the timeout code loses by construction: the timeout kill sends
SIGTERM, the child dies, the **exit watcher** observes the exit first and applies
`crashed` with its *own* generic message, and only then does the timeout path try
to apply `crashed` again — a no-op transition that emits no event. The frontend
toast fires only on a `status-changed` event carrying `status: "crashed"` and a
message, so the §9 guidance lands in the log panel and nowhere else.

A second bug rides along: when the kill could NOT be verified, `timeout_message`
appends "press Stop to retry" — but the status being applied is `crashed`, where
§6 refuses Stop and the card renders a **Run** button. The user is told to press
a button that does not exist.

## Current state

Files:

- `src-tauri/src/run.rs` — the §9 run sequence. `on_ready_timeout` (line ~441)
  is the timeout path; `timeout_message` (line ~328) builds the toast text;
  `await_ready_then_hand_off` (line ~420) dispatches the three ready outcomes.
- `src-tauri/src/process.rs` — `spawn_exit_watcher` (line ~1356) observes the
  child exit; `ProjectRuntime` (fields around lines 120–200) holds `user_stop`
  and the kill bookkeeping; `observe_child_exit` (line ~263) returns the
  user-stop flag; `take_kill_target` (line ~280).
- `src/store.ts` — lines ~131-133 fire the toast:

```ts
  if (payload.status === "crashed" && payload.message) {
    setToast(payload.message);
  }
```

The race, in code as of `91be38f`. `on_ready_timeout` (run.rs ~441):

```rust
    let target = {
        let state = app.state::<AppState>();
        let mut runtime = state.runtime.lock().await;
        runtime
            .entry(project.id.clone())
            .or_default()
            .take_kill_target()
    };

    kill_then_crash(
        async {
            let outcome = process::kill_tree(target).await;
            ...
        },
        |death_confirmed| async move {
            let message = timeout_message(project.port, project.ready_timeout_sec, death_confirmed);
            process::append_system(app, &project.id, message.clone()).await;
            let _ = apply(app, &project.id, Trigger::Failed, Some(message)).await;
        },
    )
    .await;
```

The exit watcher (process.rs, inside `spawn_exit_watcher`, ~1409-1445): after
the child is reaped it reads `(entry.observe_child_exit(), entry.status)` under
one lock, and **if `user_stop` is false** it applies
`Trigger::ChildExit { user_stop: false }` with its own message
(`crate::run::exit_message(...)` — the "was terminated without ever answering"
diagnosis), **then** sends the `exited` watch signal. `kill_tree` awaits that
signal, so by the time `on_ready_timeout`'s closure runs, the status is already
`Crashed`. In `run.rs`, `next_status(Crashed, Trigger::Failed)` returns
`Ok(Crashed)` (the "already settled" row, ~line 122-129), and `apply_with`
(~line 180) emits `status-changed` only when `from != to`. Result: no event, no
toast.

`timeout_message` (run.rs ~328-340):

```rust
pub fn timeout_message(port: u16, ready_timeout_sec: u32, death_confirmed: bool) -> String {
    let base = format!(
        "Server didn't answer on port {port} within {ready_timeout_sec} s, so it was stopped. ..."
    );
    if death_confirmed {
        base
    } else {
        format!("{base} Some of its processes could not be confirmed dead — press Stop to retry.")
    }
}
```

The mechanism to reuse (do NOT invent a new one): `ProjectRuntime.user_stop` is
exactly the "someone else owns this exit's announcement" flag. §6 (SPEC.md) says
a child exit with the flag set holds the status; the flag-setter then announces
the outcome after verification. The Stop path (`stop_project` → `claim_stop`,
process.rs ~269) sets it before killing; the timeout path does not — that is the
bug.

Conventions: every status transition goes through `run::apply`/`apply_with` and
the §6 `next_status` table — never set `entry.status` directly. Comments in this
repo cite the SPEC section they implement; match that style.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| Rust check (Windows) | `PATH="/opt/homebrew/opt/llvm/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc` | exit 0 |
| Rust tests | `cargo test --manifest-path src-tauri/Cargo.toml` | all pass |
| Acceptance tests | `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture --test-threads=1` | 3 pass |
| TypeScript | `npx tsc --noEmit` | exit 0 |
| Frontend build | `npm run build` | exit 0 |

(`cargo` lives in `~/.cargo/bin` — prefix `PATH="$HOME/.cargo/bin:$PATH"` if not found.)

## Scope

**In scope** (the only files you should modify):
- `src-tauri/src/run.rs`
- `src-tauri/src/process.rs` (only if the flag-set requires a small helper on `ProjectRuntime`)

**Out of scope** (do NOT touch):
- `src/store.ts`, `src/components/*` — the frontend toast logic is correct; the
  bug is that the event never fires.
- `src-tauri/src/process.rs` `kill_tree` / verification internals — the kill is
  correct; only who *announces* afterwards changes.
- The §6 `next_status` table in run.rs — no new transitions. SPEC.md §6 says a
  ready-timeout lands in `crashed`; this plan keeps that literal, including for
  the unverified-kill case (see step 3).
- The §7 command/event API — frozen.

## Git workflow

- Work on `main` (this repo's convention — see `git log`).
- One commit: `Fix ready-timeout crash ownership so the §9 step 7 toast is delivered`

## Steps

### Step 1: Claim exit ownership before the timeout kill

In `run.rs` `on_ready_timeout`, inside the existing lock block that calls
`take_kill_target()`, also set the ownership flag — the same field the Stop path
sets (`user_stop = true` on `ProjectRuntime`). If the field is private to
process.rs's `impl ProjectRuntime`, add a small method (e.g.
`claim_exit_ownership(&mut self)`) next to `claim_stop` rather than making the
field pub. Add a comment citing this plan's reason: the exit watcher must hold
its announcement so the timeout path can deliver the §9 step 7 message.

With the flag set, the exit watcher's `observe_child_exit()` returns true, so it
applies nothing and the status is still `Starting` when the timeout path's
closure runs: `next_status(Starting, Trigger::Failed)` → `Crashed`, a real
transition, which emits `status-changed` with the timeout message → the toast
fires.

**Verify**: `cargo check --manifest-path src-tauri/Cargo.toml` → exit 0.

### Step 2: Guard the race where the child exits before the timeout claims it

`await_ready` can return `TimedOut` in the same poll window where the child
dies. If the exit watcher already applied `crashed` (flag was still false at
that moment), `on_ready_timeout`'s `apply(..., Trigger::Failed, ...)` is a
no-op from `Crashed` — acceptable, the watcher's diagnosis was accurate and
was delivered. Confirm this path emits exactly one `crashed` event (the
watcher's). No code change expected; add a short comment in `on_ready_timeout`
stating the invariant: "if the watcher won the race before we claimed
ownership, its announcement stands and ours is a silent no-op."

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass
(existing suite: the §6 table tests still hold).

### Step 3: Fix the unverified-kill wording to an action that exists

In `timeout_message`, the `death_confirmed: false` branch currently says
"press Stop to retry". The status being applied is `crashed`; §6 refuses Stop
from `crashed` and the card shows Run. Replace the sentence with guidance that
matches the real UI, e.g.:

```
Some of its processes could not be confirmed dead. Run will refuse to start
while the port is still held and will name the process holding it.
```

(The §9 step 1 pre-check really does this — see `run_project`'s port pre-check
and `port_owner` lookup.) Update the existing unit test
`a_timeout_whose_kill_could_not_be_verified_says_so` (run.rs tests) to assert
the new wording and to assert the OLD wording is gone.

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass,
including the updated wording test.

### Step 4: Add a regression test for the delivered toast

In run.rs's test module, add a test asserting the ownership mechanics at the
state-machine level (no live Tauri app needed — model after the existing
`the_timeout_kills_the_tree_before_it_says_crashed` test):

- With the ownership flag set, `next_status(Starting, Trigger::ChildExit { user_stop: true })`
  holds `Starting` (the watcher announces nothing), and
  `next_status(Starting, Trigger::Failed)` → `Crashed` — i.e. the timeout path's
  transition is real and will emit.
- Without the flag, `next_status(Starting, Trigger::ChildExit { user_stop: false })`
  → `Crashed`, and a following `Trigger::Failed` from `Crashed` is `Ok(Crashed)`
  (from == to, no emit) — pin the no-toast case as the documented loser of the race.

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass with
the new test.

### Step 5: Manual acceptance check (SPEC §15 test 7's user-visible half)

Run the app (`PATH="$HOME/.cargo/bin:$PATH" npm run tauri dev`). Register or
seed a project whose pinned port nothing will answer (e.g. command
`node -e 'setInterval(()=>{},1e3)'`, port 39999, readyTimeoutSec 5 — edit
`~/Library/Application Support/com.hangar.app/projects.json` by hand while the
app is closed). Click Run, wait ~6 s.

**Verify**: a toast appears containing "raise Ready timeout in Edit" and the
card shows `crashed`. Record what you saw in your report.

### Step 6: Run every gate, then commit

**Verify**: all six commands in "Commands you will need" green, plus
`git status` shows only in-scope files modified.

## Test plan

- Updated: `a_timeout_whose_kill_could_not_be_verified_says_so` (new wording,
  old wording absent).
- New: the step 4 ownership/race test.
- Existing acceptance test `a_ready_timeout_kills_the_tree_and_leaves_no_orphans`
  must still pass under `--ignored --test-threads=1` — the kill ordering is
  untouched.

## Done criteria

- [ ] All six gate commands exit 0 / pass
- [ ] `grep -n "press Stop to retry" src-tauri/src/run.rs` → no matches outside test assertions of absence
- [ ] The step 5 manual check observed the §9 toast and is recorded in the report
- [ ] No files outside the in-scope list modified
- [ ] `plans/README.md` status row for 007 updated

## STOP conditions

Stop and report back if:

- The excerpts above don't match the live code (drift).
- Setting the ownership flag makes any existing §6 test fail — the flag may be
  load-bearing somewhere this plan didn't account for; do not weaken a test to
  pass.
- You find the exit watcher reads the flag BEFORE `on_ready_timeout` can set it
  in some interleaving other than step 2's (i.e. a real hole, not the accepted
  race) — report the interleaving.
- Delivering the toast appears to require a frontend change — it must not.

## Maintenance notes

- Plan 006 (M6) moves the run entry point to `updating`; the ownership claim in
  `on_ready_timeout` is phase-agnostic (it keys on the flag, not the status) and
  should survive, but 006's reviewer should re-check the timeout path.
- Reviewer should scrutinize: the flag is set under the SAME lock acquisition
  that takes the kill target (no window between them), and step 3's wording
  matches what `run_project`'s pre-check actually does.
- Deferred deliberately: routing unverified timeout kills to `stop-failed`
  instead of `crashed` — that would amend §6 and needs a spec decision first.
