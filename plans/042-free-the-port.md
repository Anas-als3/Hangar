# Plan 042: "Free the port" — the one authorised signal to a process Hangar did not spawn

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "fn get_port_status" src-tauri/src/commands.rs && grep -n "same_user\|sameUser" src-tauri/src/commands.rs src/types.ts`
> All must exist. If not, plan 041 has not merged — **STOP**.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. **Your reviewer runs `npm run build` and the bundle.**

## Status

- **Priority**: P2 — maintainer-requested, and explicitly chosen over the safer
  copy-only option after being shown the risk
- **Effort**: M
- **Risk**: **HIGH — this is the only code in Hangar that signals a process it
  did not spawn.** Every gate below is load-bearing.
- **Depends on**: plan 041 (DONE), SPEC.md §9 step 1 amendment (ratified 2026-08-10)
- **Category**: feature
- **Planned at**: 2026-08-10

## Why this matters, and why it is dangerous

Twice in one day a foreign process held a port and the §9 step 1 pre-check
refused the Run, named the PID, and left the maintainer to open a terminal.
They asked for a button. They were shown the counter-evidence — **on this very
machine, Hangar's own dev server was orphaned to PID 1, structurally identical
to the "obviously stuck" process any heuristic would target, and OrbStack holds
`5432`/`6379`/`5672`, one mistyped port away from a confirmed kill** — and chose
to build it anyway, with rails.

So: build it, and build every rail.

**Read SPEC.md §9 step 1 in full before writing a line.** It is the authority,
it was amended for this plan, and it deliberately keeps the *reason* the ban
existed in the text above the exception.

## The gates — all of them, or the button does not ship

`free_port(projectId, pid)` must refuse unless **every** one holds. Each is a
build gate, and each is testable.

| # | Gate | Why |
|---|---|---|
| 1 | Exactly **one** listening PID on that port | Two processes on `127.0.0.1:P` and `[::1]:P` is legal; the named PID may not even be the blocker |
| 2 | The holder runs as **the current user** | Never root, never another account |
| 3 | The project is **not** one Hangar is managing | Those route to Stop — §8 is the only path that may touch our own trees |
| 4 | The **full command line was read** | A truncated process name is not something a person can authorise a kill from. If `ps` gave nothing, the action is not offered **and** the command refuses |
| 5 | Identity **re-verified inside the signalling call** | PIDs are reused between the lookup and the click |
| 6 | **Positive PID only** | Never a process group, never a negative pid |
| 7 | **SIGTERM** first | SIGKILL is a separate, separately confirmed action |
| 8 | **Unix only** | On Windows the command returns an error; the UI does not offer it |

### Gate 6 deserves its own paragraph

`process.rs`'s existing `signal_group(pgid, signal)` calls
`libc::kill(-pgid, signal)` — it **negates the pid on purpose** to address a
whole process group. That is correct for Hangar's own trees and catastrophic
here: it would signal every process in a stranger's group.

**Do not call `signal_group` from this path, and do not add a boolean parameter
to it.** Write a separate function that takes a positive pid and calls
`libc::kill(pid, SIGTERM)` with no negation, and make the difference visible in
the name (e.g. `signal_one_process`). A reviewer must be able to see at a glance
which one negates.

### Gate 5 deserves its own paragraph

Between the panel's snapshot and the user's click, the PID may have exited and
been reused. Re-verify **inside** `free_port`, immediately before signalling:

- the same PID is still listening on that port (re-run the lookup), **and**
- its `ps -o lstart=` string is **byte-identical** to the one shown, **and**
- `listenerCount` is still exactly 1.

Any mismatch → return `Err` with a message saying what changed, and **signal
nothing**. This is the closest we can get to closing the TOCTOU window on
macOS, which has no `pidfd_send_signal`; the residual is "a PID reused within
the same wall-clock second by a process whose start time formats identically",
which is vanishingly unlikely and not closeable from userspace.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --all-targets` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass (baseline 134 — **run it first and report what you observe**) |
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run build`, `npm run build:app`, `npm run install:app`,
`npm run verify`, or `npm run test:acceptance`. Keep every Write/Edit under ~60
lines and commit after each.

**Never test this by signalling a real process.** Do not `kill` anything on this
machine. Unit-test the gate predicates and the parsers; the signalling call
itself is verified by the maintainer by hand.

## Scope

**In scope**:
- `src-tauri/src/process.rs` — `signal_one_process`, and a re-verification helper
- `src-tauri/src/commands.rs` — `free_port`, its gates, registration
- `src/types.ts`, `src/api.ts`, `src/store.ts` — the one new call
- `src/components/PortsPanel.tsx` — the button and its confirm

**Out of scope** (do NOT touch):
- `signal_group`, `stop_project`, `stop_all`, the §8 kill paths, `kill_pid`, the
  §6 state machine, `run.rs`. **Nothing about how Hangar kills its own trees
  changes in this plan.**
- `get_port_status` and the parsers plan 041 added — reuse them.
- `parse_lsof_owner`.
- Any escalation to SIGKILL. §9 step 1 says it is a *separate, separately
  confirmed* action; this plan ships **SIGTERM only**. If the port is still held
  afterwards, the panel says so and the user decides.
- Chaining a Run onto the confirm. §9 step 1 forbids it by name.
- Any new dependency (`libc` is already a dependency — check before assuming).

## Git workflow

- One commit per step: `Free port: <what>`.

## Steps

### Step 1: The positive-pid signal

In `process.rs`, add a function that signals **one** process by positive pid.

Requirements:
- Takes `pid: u32`, converts with `i32::try_from`, and calls
  `libc::kill(pid, libc::SIGTERM)` — **no negation anywhere in the function**.
- `#[cfg(unix)]`. Provide a `#[cfg(windows)]` counterpart that returns an error
  saying the action is unavailable on Windows (§9 step 1).
- A doc comment that names `signal_group` and states the difference explicitly:
  that one negates to address a group and is for **our own** trees; this one
  never negates and is the only path permitted to touch a foreign process.

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass;
`grep -n "libc::kill" src-tauri/src/process.rs` → exactly two call sites, and
**only** `signal_group`'s negates. Report both lines.

### Step 2: The gate predicate, as a pure function

Write a pure function that decides whether a free-port request is allowed, given
the current `PortStatus` for that project, the project's `Status`, and the
`startedAt` the caller claims to have shown the user. It returns `Ok(pid)` or
`Err(reason)`.

Keep it free of `State`, `AppHandle` and I/O so it is unit-testable — the same
shape `guard_update` uses in `commands.rs`, and for the same reason.

It must enforce gates 1, 2, 3 and 4 and the start-time match from gate 5.

**Verify**: `cargo check --all-targets` → 0.

### Step 3: Tests for every gate — write these before wiring the command

One test per refusal, each asserting the specific `Err`:

1. `listenerCount == 2` → refused
2. `sameUser: Some(false)` → refused
3. `sameUser: None` (unknown) → refused — an unknown owner is not a permission
4. project status is `running` → refused (routes to Stop)
5. project status is `starting`/`installing`/`updating`/`stopping` → refused
6. `holder.command` is `None` → refused
7. `startedAt` differs from the claim → refused
8. `holder: None` while busy → refused
9. Every gate satisfied, project `stopped` → **allowed**, returns the pid

**Verify**: `cargo test` → all pass; report the new total. Then delete gate 3's
check from the predicate, confirm test 4 **fails**, restore. **Report both
outcomes** — a gate that cannot fail a test is not a gate.

### Step 4: The command

`free_port(project_id: String, pid: u32)`:

1. Re-run `get_port_status`'s per-project probe for **that one project** — fresh,
   not the panel's snapshot.
2. Run the predicate. On `Err`, return it verbatim; **signal nothing**.
3. Assert the freshly-probed pid equals the `pid` argument. If not, return an
   error saying the owner changed. This is gate 5's second half.
4. Signal with `signal_one_process`.
5. **Re-probe the port** and return honestly. If it still accepts, the message
   must say so — do not claim success you did not verify. §8's honesty rule.
6. Append a `system` log line to that project's buffer recording what was
   signalled: pid, command line, and the outcome.

Register it in `main.rs`. Same lock discipline as `get_port_status`: snapshot,
drop, then probe.

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass.

### Step 5: The UI — the maintainer's own requirements

In `PortsPanel.tsx`, on a row in the **`In use — not managed by Hangar`** state
**only**, and only when the backend indicates all gates pass:

- The button sits **in the row's trailing corner**, after `Copy kill <pid>`, and
  is styled quieter than `Copy` — not a primary action. It is the one thing on
  this panel that is hard to undo.
- Label: **`Free the port`**. The word **Stop must never appear on it**, and
  **Free must never appear on a project's Stop button** — the two verbs mean
  different things and one is reversible.
- Clicking opens a confirm that **names the port in its question**, exactly as
  the maintainer asked:

```
Free port 5173?

This sends SIGTERM to one process. Hangar did not start it.

  node · PID 57140
  started Mon Aug 10 13:53:48 2026
  /private/tmp/…/scratchpad/fix-a11y/node_modules/.bin/vite
  its parent has exited — nothing is supervising it

              [ Cancel ]   [ Free the port ]
```

- The confirm **cannot render without the full command line** (gate 4).
- On success, refresh the panel and toast the outcome with the **neutral** tone
  — including the "still held" case, which is not an error, just a fact.
- **No auto-Run afterwards.** §9 step 1 forbids it.
- Use the existing dialog shell idiom (`AddEditDialog`'s), not a
  `window.confirm`.

**Verify**: `npm run typecheck` → 0.

### Step 6: Self-check

Report each:

- `grep -n "libc::kill" src-tauri/src/process.rs` → two sites; only `signal_group` negates.
- `grep -rn "signal_group" src-tauri/src/commands.rs` → **no match** (the foreign path never reaches it).
- `grep -n "SIGKILL" src-tauri/src/` → **no match**.
- `grep -n "Free the port\|Stop" src/components/PortsPanel.tsx` → the two verbs never share a button.
- `git status --short` → only in-scope files.

**Verify**: all three gates green.

## Test plan

Steps 3's nine gate tests are the machine-checkable part, plus step 3's
mutation check.

**Manual, by the maintainer — and this one must be done deliberately:**

- Start a dev server outside Hangar on a registered port. Open Ports. The row
  offers **Free the port**. Click it, read the confirm, cancel → nothing happens
  and the process is still alive.
- Do it again and confirm → the process dies, the row flips to `Free` after the
  panel refreshes.
- Run a project *through Hangar*, then open Ports → that row offers **Stop**,
  never **Free the port**.
- With two processes deliberately bound to the same port (one on `127.0.0.1`,
  one on `[::1]`) → the row names neither and offers no action.

## Done criteria

- [ ] All three gates green; report `cargo test` before/after
- [ ] Nine gate tests, plus the mutation check reported
- [ ] `libc::kill` is called from exactly two places and only one negates
- [ ] No SIGKILL anywhere; no auto-Run; no `signal_group` in the foreign path
- [ ] The confirm names the port and cannot render without a command line
- [ ] `plans/README.md` status row for 042 updated

## STOP conditions

Stop and report back if:

- You are tempted to reuse `signal_group` with a flag. Write the separate
  function — the whole point is that a reviewer can see which one negates.
- Any gate seems unenforceable with the data `PortStatus` carries. Report it;
  shipping the button with a gate missing is not an option.
- The re-verification cannot happen inside the same call that signals. That is
  gate 5 and the button does not ship without it.
- You find yourself killing a real process to test. Do not. Unit-test the
  predicates; the maintainer verifies the signal by hand.

## Maintenance notes

- **This is the only place in Hangar that signals a process it did not spawn.**
  If a second one ever appears, that is a §9 step 1 amendment, not a refactor.
- The gates live in a pure predicate on purpose: they are the security boundary,
  and a security boundary that needs an `AppHandle` to test is a security
  boundary nobody tests.
- The residual risk, stated plainly and not closeable on macOS: a PID reused
  within the same wall-clock second by a process whose `lstart` string formats
  identically would pass gate 5. There is no `pidfd_send_signal` on this
  platform.
- SIGKILL was deliberately left out. If the maintainer finds SIGTERM
  insufficient in practice, that is a second confirmed action per §9 step 1 —
  and it needs its own plan, not a flag on this one.
