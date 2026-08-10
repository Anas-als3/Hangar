# Plan 014: Un-brick `stop-failed` when the port is held by a process that is not ours

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `grep -n "fn stop_is_verified\|fn port_listeners\|fn ps_enrich" src-tauri/src/process.rs`
> All three must exist. Re-anchor by **symbol name**, never line number.
>
> **Re-verified 2026-08-10 at commit `5675a29`.** The finding still holds —
> `stop_is_verified(death_confirmed, port_still_answers)` is still
> `death_confirmed && !port_still_answers`, so any listener on the pinned port
> fails the stop, including one Hangar never spawned.
>
> **But this plan got substantially cheaper, and you must use what now exists
> rather than building attribution from scratch.** Plans 041 and 042 shipped
> the machinery this plan was going to have to invent:
>
> - `process::port_listeners(port, env) -> Vec<PortListener>` — every distinct
>   listening PID on a port, with its user, from one `lsof`.
> - `process::parse_lsof_all_listeners` — the all-rows parser, unit-tested.
>   Note it exists *because* the older `parse_lsof_owner` returns only the first
>   row, which is not good enough when attribution matters.
> - `process::ps_enrich(pids, env) -> HashMap<u32, PsInfo>` — one batched `ps`
>   giving ppid, start time and full command line.
>
> Use those. Do **not** add a second lookup path, and do **not** modify
> `parse_lsof_owner` — the toast path and its tests depend on its first-row
> behaviour.
>
> Two further constraints from that work, which apply here unchanged:
> **snapshot under the lock, drop it, then run `lsof`** (plan 010 made this a
> rule, and `get_port_status` is the reference shape); and this plan **sends no
> signal to anything** — attribution only. `free_port` (plan 042) is the only
> code permitted to signal a foreign process, and it is not involved here.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED — reinterprets a §8 verification rule; the maintainer approved
  the direction on 2026-08-06 (this plan's selection), but the exact deviation
  must stay inside what "Why this matters" argues.
- **Depends on**: plans/007-timeout-toast-ownership.md (both edit the stop/timeout seam in run.rs; land 007 first)
- **Category**: bug
- **Planned at**: commit `91be38f`, 2026-08-06

## Why this matters

SPEC.md §8 verification is "process death first, then the port". The port
check exists to catch **our own survivors** — leaked children that never
listen would pass a port-only check, so port-free is demanded as the second
signal. But the implementation treats ANY listener on the pinned port as a
failed stop, including processes Hangar never spawned. Consequence: kill a
project whose port has meanwhile been taken by something else (the user's own
manual server, a stale container, another app grabbing 3000), and the card is
bricked — verification finds the port busy → `stop-failed`; §6 refuses Run
from `stop-failed`; every Stop retry re-confirms the already-dead group,
re-finds the foreign listener, and fails again, forever. The only recovery is
restarting Hangar.

The fix honors §8's *intent*: when process death IS confirmed and the port
still answers, attribute the port. If the listener is provably not part of the
killed tree, the stop succeeded — report `stopped` with a warning naming the
foreign owner. Only an unattributable or same-tree listener stays
`stop-failed`. This is a documented deviation from §8's letter (CLAUDE.md
sanctions exactly this: keep the intent, note the deviation in a code
comment).

A second, related tightening: the retained kill primitive (`kill_pid`)
survives until "a verified stop or the next Run". Once `confirm_group_death`
has returned true, keeping the pid armed serves nothing — the group is gone —
and SPEC.md §16's own rule warns "never kill on PID match alone — PIDs are
reused". Dropping the pid at death-confirmation (while keeping the
`had_child` fact for honest reporting) removes the recycled-pgid hazard on
later retries.

## Current state

`src-tauri/src/run.rs` (symbols; line numbers approximate at `91be38f`):

- `stop_project` (~726) → `finish_stop` (~826): after `kill_tree`, when
  `outcome.death_confirmed`, it probes `process::port_accepts(port)` and feeds
  `process::stop_is_verified(death_confirmed, port_still_answers)`; on false it
  applies `Trigger::KillVerificationFailed` → `stop-failed` with the message
  "port {port} is still accepting connections" and restores the kill target
  for the retry (`restore_kill_target`).
- `next_status` (~91): `Stop` is legal from `StopFailed` (retry); `Run` is NOT.
- The §9 step 1 owner lookup already exists and is read-only:
  `process::port_owner(port, &env)` (process.rs ~563) returning
  `Option<PortOwner { name, pid }>`, built on `lsof -nP -iTCP:<port>
  -sTCP:LISTEN` (Unix) / `netstat`+`tasklist` (Windows), 2 s timeout, parsers
  unit-tested.
- `process::stop_is_verified` (process.rs ~869):

```rust
pub fn stop_is_verified(death_confirmed: bool, port_still_answers: bool) -> bool {
    death_confirmed && !port_still_answers
}
```

- `ProjectRuntime` (process.rs ~120): `kill_pid` ("Cleared in exactly two
  places: a *verified* stop, and the start of the next Run"), `had_child`
  semantics via `child_registered`, `take_kill_target` (~280),
  `clear_kill_target` (~296), `restore_kill_target` (~308).
- Attribution data available at the decision point: on Unix the killed
  process-GROUP id is `KillTarget.pid` (pgid); a foreign owner's pgid can be
  read with `ps -o pgid= -p <pid>` through the ONE spawn helper — or more
  simply: after `confirm_group_death(pgid)` returned true, ANY current
  listener is by definition not in our group (the group is empty). That
  simpler argument is the one to use — see step 2. On Windows the equivalent:
  `TerminateJobObject` succeeded / job process-count is 0, so a listener is
  outside the job.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass |
| Acceptance | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture --test-threads=1` | 3 pass |
| Windows check | `PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc` | exit 0 |
| TypeScript / build | `npx tsc --noEmit && npm run build` | exit 0 |

## Scope

**In scope**:
- `src-tauri/src/run.rs` (the `finish_stop` verification branch)
- `src-tauri/src/process.rs` (`stop_is_verified` signature/semantics, the
  death-confirmed pid retirement, their tests)

**Out of scope** (do NOT touch):
- `next_status` — NO new §6 transitions. The escape works entirely through the
  existing `StopConfirmed` edge; `stop-failed` still exists for the honest
  failures.
- The frontend — `stopped` + a system log line + the toast message from the
  command's `Ok`/`Err` already render correctly.
- The §9 step 1 pre-check and `port_owner` internals.

## Git workflow

- Work on `main`. One commit:
  `Attribute foreign port owners after a confirmed kill instead of bricking stop-failed`

## Steps

### Step 1: Retire the kill pid at death confirmation

In the kill/verification path: once death is confirmed
(`confirm_group_death` true on Unix; job-count-zero/terminate-success on
Windows), the retained `kill_pid` must not survive to a later retry — clear it
at the same point the outcome is built (or in `finish_stop` when
`outcome.death_confirmed`, whichever keeps the single-writer discipline —
follow where `clear_kill_target` is already called for the fully-verified
case and extend it to the death-confirmed-but-port-busy case, PRESERVING
`had_child`/`child_registered` so reporting stays honest). Add the §16
"PIDs are reused" citation as the comment.

Consequence to preserve: a retry Stop after this clearing, if it ever runs,
takes the nothing-to-signal path with `had_child` still true — which reports
death **un**confirmed... that is wrong for this case. So the clearing MUST be
paired with step 2 (the same branch settles the status), such that no retry
path needs the pid afterwards. If you find an interleaving where the pid is
cleared but the status remains `stop-failed`, STOP — the two steps have to
land as one atomic behaviour change.

**Verify**: `cargo check` → exit 0 (behaviour asserted in step 3's tests).

### Step 2: The attribution branch in finish_stop

In the `death_confirmed && port_still_answers` case (today: straight to
`stop-failed`), insert:

1. Run the read-only `process::port_owner(port, &env)` lookup (get `env` the
   same way `run_project`'s pre-check does — `state.dev_env.get().await`).
2. `Some(owner)` → the group/job is confirmed empty, so this listener is
   provably not ours. Append a system line:
   `"stopped — the process tree is gone, but port {port} is now held by {owner} (not started by Hangar)"`,
   apply `Trigger::StopConfirmed` (the existing verified-stop edge, clearing
   the kill target as that path already does), and return `Ok(())`.
3. `None` (lookup failed/timed out/nothing parseable) → keep TODAY'S behaviour
   exactly: `stop-failed` + restore the kill target + the existing message.
   Honesty beats convenience when attribution is unavailable.

Write the deviation comment at the branch, citing §8's intent ("the port check
exists to catch OUR survivors; a foreign owner after a confirmed empty
group/job satisfies that intent") and CLAUDE.md's deviation rule.

Note: with step 1, case 3's restored target has `kill_pid: None` +
`had_child: true` — the retry will report "death cannot be confirmed" via the
existing `nothing_to_signal(had_child=true, ...)` path rather than signalling
a recycled pid. That IS the §16-safe behaviour; update the message expectation
in any affected test accordingly.

**Verify**: `cargo test` → all pass (existing stop tests may need their
expectations updated ONLY where this plan's semantics apply — list every
changed assertion in the report).

### Step 3: Tests

In the existing test style (pure-function level — the §6/verification mapping
tests in both files are the pattern):

1. `a_foreign_port_owner_after_confirmed_death_is_a_verified_stop` — drive the
   decision function/branch with death_confirmed=true, port busy,
   owner=Some(...) → expect the `StopConfirmed` outcome. (If the branch is not
   currently a pure function, extract the decision —
   `fn settle_after_kill(death_confirmed, port_answers, owner: Option<&PortOwner>) -> Settle`
   — so it becomes testable; mirror how `stop_is_verified` is already pure.)
2. `an_unattributable_busy_port_stays_stop_failed` — owner=None → the
   stop-failed outcome, target restored.
3. `death_unconfirmed_never_consults_the_owner` — death_confirmed=false →
   stop-failed regardless of owner (the lookup must not even be consulted —
   assert by the enum shape or a flag).
4. `the_kill_pid_does_not_survive_a_confirmed_death` — after the
   death-confirmed path, `take_kill_target().pid` is None while
   `had_child` stays true.

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass,
4 new tests.

### Step 4: Manual check of the exact bricking scenario

With the app running and the seeded project `running`:
`python3 -m http.server <its-port>` in a terminal will fail (port busy — good,
that's not the scenario). Instead: pin the seeded project to port 39900, Run
it with a command that binds 39900 (`node -e 'require("http").createServer((_,r)=>r.end("ok")).listen(39900)'`),
then start a SECOND listener on a different shell on the SAME port — not
possible on one interface... Practical repro: click Stop, and in the ~0-1 s
between SIGTERM landing and the port probe, start
`python3 -m http.server 39900`. Timing is fiddly; if you cannot land it in
three tries, substitute: temporarily set the project's port in projects.json
to one ALREADY held by something you started (e.g. 39901 with a python server
up), get it `running` via a second port... — if no clean repro lands, the
unit tests carry the plan; record honestly which manual path you managed.

**Verify**: when the repro lands: card returns to `stopped`, the log shows
"port 39900 is now held by python3 (PID …) (not started by Hangar)". Recorded
in the report either way.

### Step 5: All gates, then commit

**Verify**: all six commands green; only the two in-scope files modified.

## Test plan

The four tests in step 3; every §15 acceptance test still green (the ordinary
kill path — foreign-owner absent — is behaviour-identical); manual repro
best-effort per step 4.

## Done criteria

- [ ] All gates green; 4 new tests pass
- [ ] The deviation comment exists at the branch and cites §8 intent + CLAUDE.md's deviation rule (`grep -n "not started by Hangar" src-tauri/src/run.rs`)
- [ ] `stop-failed` still reachable (test 2) — the state is narrowed, not removed
- [ ] Changed test expectations enumerated in the report
- [ ] Only run.rs + process.rs modified
- [ ] `plans/README.md` status row for 014 updated

## STOP conditions

Stop and report back if:

- You cannot make steps 1+2 land as one atomic behaviour (see step 1's
  consequence paragraph) — a half state is worse than today's brick.
- The §15 acceptance tests fail after the change — the ordinary path must be
  untouched.
- Attribution on Windows turns out to need the job handle after
  `TerminateJobObject` dropped it — report; do not weaken the Unix path to
  match.
- Any §6 table test needs weakening — that means the escape leaked into the
  state machine, which is out of scope.

## Maintenance notes

- SPEC.md §16's "Unix crash recovery" and "Explicit port repin" entries both
  interact with this seam; whoever promotes them should read the deviation
  comment first.
- Reviewer should scrutinize: the owner lookup runs ONLY in the
  death-confirmed case (never as a substitute for death verification — that
  would be the §8 false proxy), and the restored-target retry path after
  step 1 reports honestly.
- Deferred deliberately: surfacing the foreign owner in a toast (the `Ok(())`
  return means today's Stop button just settles; the log line carries the
  detail) — revisit with plan 005's UI work if users miss it.
