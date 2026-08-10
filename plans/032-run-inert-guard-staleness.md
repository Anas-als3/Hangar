# Plan 032: Make the run-inert exemption actually reachable

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "is_run_inert_change\|merge_run_inert_fields" src-tauri/src/commands.rs`
> Both must exist. If they do not, plan 028 has not merged — **STOP**.

## Status

- **Priority**: P1 — a shipped feature is unreachable in its main use case, and
  a second shipped feature (notes autosave, 2026-08-09) has the same hole
- **Effort**: S
- **Risk**: MED — touches the §6 mutation guard, one of CLAUDE.md's
  highest-priority correctness requirements
- **Depends on**: plan 028 (DONE), SPEC.md §6 amendment (ratified 2026-08-10)
- **Category**: bug
- **Planned at**: commit after the §6 amendment, 2026-08-10

## Why this matters

Found by a fresh-context reviewer auditing the folders diff. Two bugs, one
cause.

### Bug 1 — the exemption cannot be reached (HIGH)

`is_run_inert_change` (`src-tauri/src/commands.rs:139-149`) normalises out
`notes` / `folder_id` / `folder_name` and compares **the whole rest of the
record** with the derived `PartialEq`. That is only correct if the caller's
payload matches the stored record in every other field.

It does not. `src-tauri/src/run.rs:815-821` runs on **every** Run, before
spawn:

```rust
let stack = registry::read_package_json(Path::new(&project.path)).stack;
let started_at = iso8601_utc(SystemTime::now());
let persist_error = {
    let mut projects = state.projects.lock().await;
    if let Some(p) = projects.iter_mut().find(|p| p.id == project.id) {
        p.last_run_at = Some(started_at);
        p.stack = Some(stack);
    }
```

The frontend never learns: `applyStatusChanged` patches only `status`, and
`loadRegistry()` fires only at mount and on window focus.

**Failure scenario.** A project is `stopped`, last run yesterday, Hangar has
focus. The user clicks Run; the backend stamps `lastRunAt = now`. During the
`updating` → `installing` → `starting` window (up to `readyTimeoutSec`, default
60 s) the user opens `⋯ → Move to folder… → New folder → Move`. The payload
carries **yesterday's** `lastRunAt`. `is_run_inert_change` → `false`.
`guard_update` → `Err("… is running. Stop it first.")`.

The user is told to stop a project in order to file it into a folder — exactly
what SPEC.md §6's amended bullet says must not happen.

**Plan 028 fixed the write side and not the read side.** `merge_run_inert_fields`
exists precisely because of this staleness, and its own doc comment says so —
but the guard two lines above it consumes the same stale payload and rejects
before the merge is ever reached. Notes autosave (2026-08-09) has had the same
hole since it shipped.

### Bug 2 — a full Edit rolls back app-owned fields (LOW, same cause)

`src/components/AddEditDialog.tsx:152-156` sends `{...editing, ...payload}`,
where `editing` was captured when the dialog opened, and
`stopIfRunningWithConfirm` does not reload the registry after stopping. A
guarded update takes the whole-record replace branch
(`src-tauri/src/commands.rs:222`), so editing a project that ran this session
writes back the **pre-run** `lastRunAt` and `lastLockfileHash`. Rolling back
`lastLockfileHash` costs one spurious reinstall on the next Run; rolling back
`lastRunAt` mislabels the card's time slot.

## The rule, now in SPEC.md §6

> **App-owned fields are normalised out of that comparison too, and the caller
> can never write them.** The app-owned set is `lastRunAt` and
> `lastLockfileHash` … Both fields are preserved from the stored record on
> every write, guarded or not, so normalising them out cannot widen what a
> caller is able to change.

**`stack` is deliberately NOT in the set.** The Edit dialog re-detects it and
legitimately supplies a fresh one (plan 025). It must stay writable — but it
also must not defeat the comparison, so see step 2.

## Current state

`src-tauri/src/commands.rs:139-149`:

```rust
fn is_run_inert_change(stored: &Project, incoming: &Project) -> bool {
    let mut stored = stored.clone();
    let mut incoming = incoming.clone();
    stored.notes = None;
    incoming.notes = None;
    stored.folder_id = None;
    incoming.folder_id = None;
    stored.folder_name = None;
    incoming.folder_name = None;
    stored == incoming
}
```

`src-tauri/src/commands.rs:~205-228` — the guard call, the port check, the
merge/replace branch, and the view built from `projects[index]`.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --all-targets` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass (baseline 110 — **run it first and report what you observe**) |

**Do not run** `npm run verify`, `npm run build`, `npm run build:app`, or
`npm run test:acceptance` — a 600 s no-output watchdog has killed executor runs
here. Keep every Write/Edit under ~60 lines and commit after each.

## Scope

**In scope**:
- `src-tauri/src/commands.rs` — the comparison, the write, and new tests

**Out of scope** (do NOT touch):
- **Anything under `src/`.** Another executor is working there concurrently;
  touching it causes a merge conflict. This is a backend-only fix.
- `src-tauri/src/run.rs`. Its per-Run write is correct — §9 step 4 requires it.
  The bug is in what the guard does with the consequence, not in the write.
- `guard_mutation`, the §6 state machine, §8 kill paths, §9 run sequence.
- Any §7 command signature. §7 is FROZEN.
- `stack` — leave it writable from the payload.
- Any new dependency.

## Git workflow

- One commit per step: `Run-inert guard: <what>`.

## Steps

### Step 1: Normalise the app-owned fields out of the comparison

Extend `is_run_inert_change` to also null `last_run_at` and
`last_lockfile_hash` on both sides before comparing.

`stack` needs different treatment and is step 2 — do not touch it here.

Update the doc comment to cite SPEC.md §6's app-owned bullet and to say plainly
*why* a second normalised list is not a loosening: these are fields the backend
writes behind the frontend's back, and step 3 makes them unwritable by a caller,
so removing them from the comparison cannot widen what a caller can change.

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → report the
count.

### Step 2: Stop `stack` from defeating the comparison

`stack.detected_at` is re-stamped on every Run (`registry.rs:559`), so a stale
payload differs there too — the same failure, one field over.

`stack` must stay **writable** (the Edit dialog re-detects it), so it cannot
simply join the app-owned set. Instead:

- In the comparison only, treat `stack` as equal when the incoming value is
  `None` **or** when it differs from stored only in `detected_at`. A caller that
  genuinely changed the detected framework or libraries is still a guarded
  change.
- Implement it as a small named helper (e.g. `stack_is_unchanged_ignoring_timestamp`)
  with a doc comment, not as inline logic — a reviewer must be able to see the
  exemption's exact edge.

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → all pass.

### Step 3: Preserve app-owned fields on every write

In `update_project`, both branches must keep the stored `last_run_at` and
`last_lockfile_hash`:

- The **merge** branch already writes only the three run-inert fields, so it is
  correct as-is. Confirm and say so in your report — do not change it.
- The **replace** branch (`projects[index] = project`) must restore both fields
  from the stored record after replacing. Capture them before the write, or
  write the payload into the slot and then overwrite the two fields — either is
  fine, but add a comment naming SPEC.md §6's app-owned bullet.

The returned `ProjectView` is already built from `projects[index]`, so it needs
no change. Confirm that too.

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → all pass.

### Step 4: Tests that would have caught both bugs

Add to `commands.rs`'s test module, following `sample_project`'s style:

1. **The headline test.** Stored has `last_run_at: Some("2026-08-10T…")`;
   incoming is identical except `folder_id: Some(…)` and
   `last_run_at: Some("2026-08-05T…")` (a stale frontend copy). Assert
   `is_run_inert_change` → `true`, and `guard_update(…, Status::Starting)` →
   `Ok`. **Without step 1 this test fails** — say so in your report.
2. Same shape for `last_lockfile_hash`.
3. Same shape for a `stack` differing **only** in `detected_at`.
4. A `stack` differing in `framework` is **still guarded** while running.
5. A `port` change paired with a stale `last_run_at` is **still guarded** — the
   normalisation must not become a smuggling route.
6. The replace branch preserves app-owned fields: build the guarded-write path's
   logic as a testable free function if it is not already, and assert that a
   payload with `last_run_at: None` and `last_lockfile_hash: None` leaves both
   stored values intact.

**Verify**: `cargo test` → all pass; report the new total.

### Step 5: Prove the tests are not vacuous

Temporarily revert step 1's two `None` assignments, run `cargo test`, and
confirm test 1 and test 2 **fail**. Restore, confirm they pass. **Report both
outcomes.** A test that cannot fail is not a test — this repo has shipped a
vacuous guard before.

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → all pass;
`git status --short` shows only `src-tauri/src/commands.rs`.

## Test plan

Steps 4 and 5 are the test plan. Manual check for the maintainer, which is the
scenario that motivated this plan:

- Click Run on a project. **While it is still installing or starting** (before
  the browser opens), open `⋯ → Move to folder… → New folder…` and save. It must
  succeed. Before this plan it fails with "… is running. Stop it first."
- Same window, open Notes and type. The autosave must succeed.
- After it reaches `running`, Edit it (confirm-and-stop), change nothing but the
  name, save. Its "Last run" time must not jump backwards.

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo test` passes; report before/after counts
- [ ] The vacuity check in step 5 was run and both outcomes reported
- [ ] The merge branch is unchanged; the replace branch preserves both app-owned fields
- [ ] `stack` is still writable from the payload
- [ ] Nothing under `src/` modified
- [ ] `plans/README.md` status row for 032 updated

## STOP conditions

Stop and report back if:

- Fixing this appears to need a §7 change, a new command, or a change to
  `run.rs`'s per-Run write. It needs none of those.
- You conclude `stack` should join the app-owned set. It must not — the Edit
  dialog legitimately supplies a fresh one (plan 025), and making it unwritable
  would silently break stack refresh on Edit.
- The comparison starts to look like a hand-written list of *guarded* fields.
  SPEC.md §6 forbids that: a field added later must be guarded by default.

## Maintenance notes

- The lesson is worth keeping and is not really about folders: **a guard that
  compares a caller's whole payload against server state is only as correct as
  the caller's copy is fresh.** Every field the backend writes without telling
  the frontend is a false rejection waiting to happen. When a new such field is
  added, it belongs in the app-owned set the same day.
- Plan 028 fixed the write side of this staleness and its author (me) described
  that as fixing the bug. It was half the fix. The read side sat one function
  above and went unexamined because the write-side test passed.
- If a third normalised category ever appears, that is a signal the comparison
  has outgrown "clone and compare" and deserves an explicit, tested
  field-classification function instead.
