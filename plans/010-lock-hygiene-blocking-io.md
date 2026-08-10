# Plan 010: Move blocking filesystem I/O out from under the async state locks

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `grep -n "save_projects" src-tauri/src/run.rs` and
> `sed -n '/pub async fn get_projects/,/^}/p' src-tauri/src/commands.rs`.
>
> **Re-verified 2026-08-10 at commit `e7ecada`** — both findings still hold, and
> the line numbers below have moved since this plan was written (2026-08-06).
> Work from the *shape* described, not from the old line numbers:
>
> - `run.rs` has **two** in-lock saves now, not one: the `lastRunAt` + `stack`
>   write in `run_project` (~:830-839) and the `lastLockfileHash` + `stack`
>   write in `store_lockfile_hash` (~:1076-1085). Plan 025 added the second.
>   Both are `{ let mut projects = state.projects.lock().await; …;
>   registry::save_projects(&state.config_dir, &projects).err() }` — the save,
>   and its `fsync`, happen inside the block that holds the lock.
> - `commands.rs`'s `get_projects` still takes **both** locks and then calls
>   `to_view`, which calls `Path::new(&project.path).exists()` per project.
>
> **This has become more urgent, not less.** When the plan was written,
> `get_projects` ran at startup and on window focus. Plan 038 made
> `refreshRegistryQuietly` the default post-mutation refresh, so it now also
> runs after every add, edit, remove, folder move, rename, ungroup, notes
> autosave and rejected Run — and plan 035 calls it again whenever a project
> reaches `running`.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `91be38f`, 2026-08-06

## Why this matters

SPEC.md §4 is explicit that the async state mutexes must not be held across
long operations — the kill sequences and every command need them. Two sites
violate this with *blocking filesystem I/O*:

1. `run_project` persists `lastRunAt` by calling `registry::save_projects`
   **inside** `state.projects.lock().await`. That save does
   `File::create` + `write_all` + **`sync_all()` (a real fsync)** + `rename`
   (see `registry.rs::atomic_write`, ~line 122). An fsync on a busy/full disk
   parks a tokio worker while holding the lock every Run/Stop path and the
   exit watcher need.
2. `get_projects` calls `Path::exists()` once per project while holding
   **both** the `projects` and `runtime` locks. One project whose path sits on
   an unreachable network mount or a stalled cloud-sync folder (SPEC §16 even
   anticipates these) blocks both locks for seconds — the whole app freezes,
   including kill verification.

The fix is mechanical: snapshot under the lock, do I/O after release.

## Current state

- `src-tauri/src/run.rs` ~lines 556-566 (inside `run_project`):

```rust
    // ---- §5/§6: lastRunAt is set when entering `starting` -------------------------------------
    let started_at = iso8601_utc(SystemTime::now());
    let persist_error = {
        let mut projects = state.projects.lock().await;
        if let Some(p) = projects.iter_mut().find(|p| p.id == project.id) {
            p.last_run_at = Some(started_at);
        }
        registry::save_projects(&state.config_dir, &projects).err()
    };
    if let Some(e) = persist_error {
        process::append_system(app, &project.id, format!("could not save lastRunAt: {e}")).await;
    }
```

- `src-tauri/src/commands.rs` ~lines 62-79:

```rust
fn to_view(project: &Project, runtime: &RuntimeMap) -> ProjectView {
    ProjectView {
        project: project.clone(),
        status: runtime
            .get(&project.id)
            .map(|r| r.status)
            .unwrap_or(Status::Stopped),
        path_exists: Path::new(&project.path).exists(),
    }
}

#[tauri::command]
pub async fn get_projects(state: State<'_, AppState>) -> Result<Vec<ProjectView>, String> {
    let projects = state.projects.lock().await;
    let runtime = state.runtime.lock().await;
    // Array order is the display order — no sorting, ever (SPEC.md §11).
    Ok(projects.iter().map(|p| to_view(p, &runtime)).collect())
}
```

- `AppState` (commands.rs ~line 21): `projects: Mutex<Vec<Project>>`,
  `runtime: Mutex<RuntimeMap>`, `config_dir: PathBuf` — all
  `tokio::sync::Mutex`.
- `registry::save_projects(&Path, &[Project]) -> Result<(), String>` — takes a
  slice, so it works on a clone unchanged.
- `set_settings` (commands.rs ~line 141) also saves under the `settings` lock;
  settings are one small struct and the lock is uncontended — treat it in
  step 3 for consistency but it is not the hot path.

Convention: comments cite the SPEC section they implement; keep the §4
citation on the new shape. `tauri::async_runtime::spawn_blocking` is available
(tokio's under the hood) — use it for the fsync-bearing save.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass |
| Acceptance | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture --test-threads=1` | 3 pass |
| Windows check | `PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc` | exit 0 |
| TypeScript | `npx tsc --noEmit` | exit 0 |
| Frontend build | `npm run build` | exit 0 |

## Scope

**In scope** (the only files you should modify):
- `src-tauri/src/run.rs` (the lastRunAt block only)
- `src-tauri/src/commands.rs` (`get_projects`/`to_view`, `set_settings`, `AppState` if a save-serialization mutex is added)

**Out of scope** (do NOT touch):
- `src-tauri/src/registry.rs` — `atomic_write`'s temp-file/fsync behaviour is
  plan 012's scope. Call it from a better place; do not change it.
- `src-tauri/src/process.rs` — no kill-path changes.
- The frontend.

## Git workflow

- Work on `main`. One commit:
  `Move blocking registry I/O and path stats out from under the async state locks`

## Steps

### Step 1: lastRunAt — mutate under the lock, save after it

Reshape the run.rs block to:

1. Lock `projects`, update `last_run_at`, **clone the Vec**
   (`projects.clone()`), drop the guard (end of block).
2. Acquire a write-serialization guard: add `save_lock: tokio::sync::Mutex<()>`
   to `AppState` (one line + init) and hold it across the save so two
   concurrent saves cannot interleave their temp-file/rename pairs.
3. Run the save on the blocking pool:
   `tauri::async_runtime::spawn_blocking(move || registry::save_projects(&config_dir, &snapshot))`
   and `.await` the join handle while holding ONLY `save_lock` (never
   `projects`).
4. Keep the existing "could not save lastRunAt" system-log line on error,
   including a join-error arm (`Err(join)` → same log path).

Comment the shape with: "§4: the projects lock is never held across blocking
I/O; save_lock serializes writers instead."

**Verify**: `cargo check` → exit 0; `cargo test` → all pass.

### Step 2: get_projects — stat after both guards drop

Reshape: under the two locks, build a snapshot
`Vec<(Project, Status)>` (project clone + status with the existing
`unwrap_or(Stopped)` logic), preserving array order; drop both guards; then map
the snapshot through `Path::new(&p.path).exists()` into `ProjectView`s.

Implementation detail: split `to_view` so the stat is separable — e.g.
`fn to_view(project: Project, status: Status, path_exists: bool) -> ProjectView`
or inline the construction; keep plan 008's tests compiling if they landed
first (adjust their call shape mechanically, not their assertions).

`Path::exists()` on the snapshot still runs on the async thread — that is
acceptable here (it no longer holds any lock, and get_projects is called once
at startup); do NOT add spawn_blocking for it, to keep the diff minimal.

**Verify**: `cargo test` → all pass (including plan 008's `to_view` tests if
present — adapt call sites only).

### Step 3: set_settings — same shape, small scale

In `set_settings`: clone the incoming settings, do
`save_settings` via `spawn_blocking` under `save_lock` (NOT under the
`settings` data lock), then write the data lock last on success. Preserves the
current behaviour where a failed save leaves the in-memory settings unchanged.

**Verify**: `cargo check` → exit 0.

### Step 4: All gates + a manual smoke

Run all six commands. Then `PATH="$HOME/.cargo/bin:$PATH" npm run tauri dev`
(or `npm run dev` if plan 009 landed), Run + Stop the seeded project once —
confirms lastRunAt still persists (check the JSON file's `lastRunAt` field
updated: `grep lastRunAt "$HOME/Library/Application Support/com.hangar.app/projects.json"`).

**Verify**: all gates green; the grep shows a fresh ISO timestamp.

## Test plan

No new tests required (the reshaping is behaviour-preserving and the §15
acceptance lane plus the manual smoke cover the integration); the REAL test is
that all 70+ existing tests and the 3 acceptance tests still pass unchanged.
If plan 008 landed first, its `to_view` tests must pass with only mechanical
call-shape adaptation.

## Done criteria

- [ ] All six gate commands green
- [ ] `grep -n "save_projects" src-tauri/src/run.rs` shows it OUTSIDE any `projects.lock()` block (visual check recorded in report)
- [ ] `get_projects` performs no `Path::exists` while a lock guard is alive (visual check recorded in report)
- [ ] Manual smoke: lastRunAt still persists after a Run
- [ ] No files outside the in-scope list modified
- [ ] `plans/README.md` status row for 010 updated

## STOP conditions

Stop and report back if:

- The excerpts don't match the live code (drift — especially if plan 007/008
  landed and moved lines).
- Any existing test fails after the reshape and the failure is not a
  mechanical call-shape change (a semantic change means the reshape broke an
  ordering assumption — report it).
- You find another `sync_all`/`Path::exists` site under a lock not listed here
  — report it as a finding; fix only the listed ones.

## Maintenance notes

- Plan 005 (`add_project`/`update_project`/`remove_project`) adds three more
  writers to `projects.json`. They MUST follow this plan's shape: mutate under
  the data lock, snapshot, save under `save_lock` on the blocking pool. Its
  reviewer should check for regressions to the old shape.
- Reviewer should scrutinize: the save error still reaches the project's log,
  and no await point sneaks between "mutate" and "snapshot" (a Stop's
  registry read between them would see the new lastRunAt — fine — but a second
  Run's would too, which is also fine; the invariant is only that saves are
  serialized).
