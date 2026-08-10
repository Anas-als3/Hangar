# Plan 028: Folders, backend half — the two fields and the run-inert guard

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat c53d767..HEAD -- src-tauri/src/registry.rs src-tauri/src/commands.rs`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P2 — maintainer-requested feature; this half also fixes a live bug
- **Effort**: M
- **Risk**: MED — touches the §6 mutation guard, which is one of CLAUDE.md's
  "highest-priority correctness requirements"
- **Depends on**: SPEC.md §5/§6/§11/§16 amendments (ratified `c53d767`)
- **Category**: feature + bug
- **Planned at**: commit `c53d767`, 2026-08-10

## Why this matters

The maintainer asked for iOS-style folders: drag one card onto another and they
group. This plan is the **backend half of the non-drag foundation** — the
persisted shape and the guard that lets a card be filed while it is running.
Plan 029 builds the UI; plan 030 adds the gesture. Nothing here renders
anything.

**Read SPEC.md §5's "Folder semantics" bullet and §6's two exception bullets
before you start.** They were amended for this and they are the authority.

The design in one line: **a folder has no record of its own.** It *is* the set
of projects sharing a `folderId`. Empty folders and dangling folders are
therefore unrepresentable, `remove_project` needs no cleanup, `projects.json`
stays a bare array, and no §7 command is added or reshaped.

### The bug this also fixes

`is_notes_only_change` (`commands.rs:134-140`) is correct, but
`update_project` then does `projects[index] = project.clone()`
(`commands.rs:187`) — a **whole-record replace from the caller's payload**.

`src-tauri/src/run.rs` persists `last_run_at` **and** a freshly detected `stack`
on every Run. The frontend's `status-changed` handler patches only `status`, and
the registry is re-fetched only on mount and window focus. So for the entire
`updating` → `installing` → `starting` window the frontend's copy of those two
fields is **stale**, and any run-inert save during that window rolls them back.
That is a live notes-autosave bug today, before folders exist. SPEC.md §6's
second exception bullet (added 2026-08-10) now forbids it in writing.

## Current state

`src-tauri/src/registry.rs:34-61` — `Project`, ending at `stack`:

```rust
    /// SPEC.md §5: detected from `package.json` dependencies — app-owned, never hand-edited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<ProjectStack>,
}
```

`src-tauri/src/registry.rs:88-107` — `NewProject`, ending the same way with
`#[serde(default)]` (no `skip_serializing_if`; it is `Deserialize` only).

`src-tauri/src/commands.rs:134-140` — the guard's comparison:

```rust
fn is_notes_only_change(stored: &Project, incoming: &Project) -> bool {
    let mut stored = stored.clone();
    let mut incoming = incoming.clone();
    stored.notes = None;
    incoming.notes = None;
    stored == incoming
}
```

`src-tauri/src/commands.rs:154-190` — `guard_update` and the replace:

```rust
fn guard_update(stored: &Project, incoming: &Project, status: Status) -> Result<(), String> {
    if is_notes_only_change(stored, incoming) {
        return Ok(());
    }
    crate::run::guard_mutation(status, &stored.name)
}
// ... in update_project:
    projects[index] = project.clone();
    registry::save_projects(&state.config_dir, &projects)?;
```

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --all-targets` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass (baseline is 104 passed, 3 ignored — **report the number you actually observe before changing anything**) |

**Do not run** `npm run verify`, `npm run build`, `npm run build:app`, or
`npm run test:acceptance` — a 600 s no-output watchdog has killed executor runs
here. Keep every Write/Edit under ~60 lines and commit after each.

## Scope

**In scope**:
- `src-tauri/src/registry.rs` — the two fields on `Project` and `NewProject`,
  and the test fixtures/assertions they break
- `src-tauri/src/commands.rs` — `add_project` carry-through, the run-inert
  guard, the merge-not-replace write, and new unit tests
- `src/types.ts` — the two matching optional properties **only** (the drift
  guard test reads this file; nothing else in `src/` is yours)
- `src-tauri/src/run.rs` — **only** its `project_fixture` if the new fields make
  it fail to compile

**Out of scope** (do NOT touch):
- Every §7 command signature. `update_project(project: Project)` already carries
  the whole record, so folders need **no new command and no reshaped one**. §7
  is FROZEN.
- Any UI file other than `src/types.ts`. No component, no store, no CSS. Plan
  029 owns all of that and is dispatched separately — touching it causes a merge
  conflict.
- `remove_project`. It needs no folder cleanup; that is the entire point of the
  no-folder-record design. If you find yourself adding cleanup there, you have
  misread the model — STOP.
- The §6 state machine, §8 kill paths, §9 run sequence, `guard_mutation` itself.
- Any migration, any startup sweep, any versioned storage wrapper. Both fields
  are optional with `skip_serializing_if`, exactly like `notes` and `stack`.
- Any new dependency. Do **not** add an id-generation crate — the frontend
  generates `folderId`, and `registry.rs`'s existing id comment explains why no
  crate was added for project ids either.

## Git workflow

- One commit per step: `Folders backend: <what>`.

## Steps

### Step 1: The two fields

In `src-tauri/src/registry.rs`, add to `Project` after `stack`:

```rust
    /// SPEC.md §5 (folders, 2026-08-10): the folder this project is filed under. Opaque and
    /// generated — NEVER derived from the name, so two folders may share a name exactly as on
    /// iOS. A folder is exactly the set of projects carrying this id: it has no record of its
    /// own, so it cannot dangle, cannot be empty, and `remove_project` needs no cleanup.
    /// Run-inert: nothing in §8's kill paths or §9's run sequence reads it (§6). NOT the
    /// project's directory on disk — that is `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    /// SPEC.md §5: the folder's display name, denormalised onto every member so no second file
    /// and no new §7 command is needed. A rename is N writes; if members ever disagree (a rename
    /// interrupted mid-way) the earliest member in array order supplies the displayed name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_name: Option<String>,
```

Mirror both on `NewProject` with `#[serde(default)]` and a one-line comment
pointing at `Project`'s. Carry them through in `add_project`
(`commands.rs:98-113`) exactly as `notes` and `stack` are carried.

Add both to `src/types.ts`'s `Project` interface as
`folderId?: string;` / `folderName?: string;`, with a comment matching that
file's style.

**Verify**: `cargo check --all-targets` → exit 0 (expect the fixture failures in
step 2 to appear here first; that is fine, fix them in step 2 and re-run).

### Step 2: Repair every exhaustive fixture and assertion

Adding fields to `Project` breaks every exhaustive struct literal and one exact
JSON assertion. Find them all:

```
grep -rn "ready_timeout_sec:" src-tauri/src/
```

Known sites: `registry.rs:173` `seed_projects`, `registry.rs:596` `sample()`,
`commands.rs:98` `add_project`, `commands.rs:324` `sample_project`,
`run.rs:1989` `project_fixture`.

Two of them need more than `None`:

1. **`registry.rs`'s `sample()` must set both to `Some(...)`.** Read the comment
   already sitting above `notes` there — it explains that
   `skip_serializing_if` omits the key for `None`, which would let the drift
   guard `every_wire_key_the_backend_emits_appears_in_types_ts`
   (`registry.rs:784`) pass **vacuously**. The same trap applies here. Use
   `Some("fld_1".into())` and `Some("Client Work".into())`.
2. **`project_view_flattens_project_and_adds_derived_fields`
   (`registry.rs:729-751`) asserts an exact JSON string** in field-declaration
   order. Extend it with `"folderId":"fld_1","folderName":"Client Work"`
   **after** `stack` and **before** `status` — the order must match the struct.

**Verify**: `cargo test` → all pass, count is baseline + 0 new. Report the
count. Then confirm the drift guard is not vacuous: temporarily delete
`folderId?: string;` from `src/types.ts`, run `cargo test`, and confirm
`every_wire_key_the_backend_emits_appears_in_types_ts` **fails**; restore it and
confirm it passes again. **Report both outcomes** — a guard that cannot fail is
not a guard.

### Step 3: Generalise the guard to the run-inert set

In `src-tauri/src/commands.rs`:

1. Rename `is_notes_only_change` → `is_run_inert_change` and null out all three
   fields (`notes`, `folder_id`, `folder_name`) on both sides before comparing.
   **Keep the clone-and-compare shape.** Do not convert it to a field list —
   the existing doc comment explains that the structural comparison is what
   makes a future field guarded by default, and SPEC.md §6 requires it.
2. Update `guard_update`'s call and both doc comments to name the run-inert set
   and cite SPEC.md §6's amended bullet.

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → all pass.

### Step 4: Merge instead of replace — the bug fix

In `update_project`, replace the unconditional whole-record write with a
branch:

- **Run-inert change** (the same predicate the guard used — compute it once
  before the guard and reuse the boolean; do not call it twice): write **only**
  `notes`, `folder_id` and `folder_name` into `projects[index]`. Leave every
  other stored field untouched.
- **Otherwise** (a guarded edit, so the project is `stopped` or `crashed` and
  nothing is being written underneath it): keep the existing whole-record
  replace.

The returned `ProjectView` must reflect **what was stored**, not the caller's
payload — after a merge, build the view from `projects[index]`, or the frontend
receives back the stale `lastRunAt`/`stack` this step exists to protect.

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → all pass.

### Step 5: Tests for both behaviours

Add to `commands.rs`'s existing test module, following `sample_project`'s style:

1. A change to `folder_id` alone is run-inert → `guard_update` returns `Ok` for
   a `Running` status.
2. A change to `folder_name` alone is run-inert → same.
3. A change to `notes` **and** `folder_id` together is run-inert → `Ok`.
4. A change to `port` alone is **still guarded** → `Err` for `Running`.
5. A change to `folder_id` **and** `port` is **still guarded** → `Err`. This is
   the one that proves the exemption cannot be used as a smuggling route.
6. **The merge test** — the reason step 4 exists. Build a stored project with
   `last_run_at: Some("2026-08-05T10:00:00Z")` and an incoming payload identical
   except `folder_id: Some(...)` and `last_run_at: None` (a stale frontend
   copy). Apply the same merge logic and assert the result keeps
   `last_run_at == Some("2026-08-05T10:00:00Z")`. If the merge logic lives
   inside the `#[tauri::command]` and cannot be reached from a test, **extract
   it into a small free function** taking `(&mut Project, Project)` — that is
   exactly why `guard_update` was pulled out as plain-data-in/`Result`-out, and
   the doc comment there says so.

**Verify**: `cargo test` → all pass; report the new total.

### Step 6: Gates

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → all pass;
`git status --short` shows only the in-scope files.

## Test plan

Covered by step 5 above. The six unit tests are the deliverable's proof; there
is no GUI in this half and no manual check a subagent could run. The
mutation-test in step 2 (delete the `types.ts` line, confirm the guard fails) is
mandatory and must be reported.

## Done criteria

- [ ] `cargo check --all-targets` exits 0
- [ ] `cargo test` passes; report before/after counts
- [ ] `folderId`/`folderName` appear in `src/types.ts` and in `sample()` as
      `Some(...)`, and the drift guard was proven to fail without them
- [ ] `is_run_inert_change` still uses clone-and-compare, not a field list
- [ ] A run-inert update writes only the three run-inert fields
- [ ] `grep -rn "folder" src-tauri/src/run.rs src-tauri/src/process.rs` finds
      nothing but the fixture — folders are invisible to §8/§9
- [ ] No new §7 command, no reshaped one, no new dependency, no migration
- [ ] `plans/README.md` status row for 028 updated

## STOP conditions

Stop and report back if:

- Anything appears to need a new command, a changed command signature, or a
  second persisted file. §7 is FROZEN and the design needs none of these.
- `remove_project` seems to need folder cleanup. It does not — a folder with no
  members ceases to exist by construction.
- The merge logic cannot be extracted into a testable free function. Report the
  constraint rather than shipping step 4 untested.
- You conclude the guard should become a list of guarded field names. SPEC.md §6
  forbids it in writing: a field added later must be guarded by **default**.

## Maintenance notes

- The run-inert set is now named in exactly two places: SPEC.md §6's bullet and
  `is_run_inert_change`. Adding a fourth run-inert field means editing both, and
  the structural comparison means forgetting is safe (the field stays guarded)
  rather than dangerous.
- The merge-not-replace rule generalises: **any** command that writes a record
  the frontend has been holding across a run is exposed to the same staleness.
  `update_project` was the only one; if another appears, it needs the same
  treatment.
- `folderId` is generated by the frontend. If a future plan ever generates it in
  Rust, it must not use the project-id scheme verbatim — read the id comment at
  `registry.rs:110` first.
