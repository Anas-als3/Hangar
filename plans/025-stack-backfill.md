# Plan 025: Make the stack badge appear for projects that already exist

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat 70cbf6f..HEAD -- src-tauri/src/run.rs src/components/AddEditDialog.tsx`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P2 — plan 023's feature is invisible for every project that
  predates it, which is currently all of them
- **Effort**: S
- **Risk**: LOW
- **Depends on**: plans/023 (DONE, merged `474a9ed`)
- **Category**: bug
- **Planned at**: commit `70cbf6f`, 2026-08-09

## Why this matters

Plan 023 shipped stack detection and a framework badge. It works — for projects
added through the dialog *after* it landed. For every project that already
existed, the badge never appears, because the two write paths both miss:

1. **The install phase.** `store_lockfile_hash` (`src-tauri/src/run.rs:1050`) is
   the only place `p.stack` is written, and it is called from exactly one site
   — `src-tauri/src/run.rs:1243`, inside `PhaseOutcome::Exited(Some(0))`, i.e.
   **only after a successful install**. Per SPEC.md §9 step 3 an install only
   runs when the lockfile hash changed or `node_modules` is absent. A project
   that is already installed and unchanged runs, starts, serves — and never
   refreshes its stack.
2. **The Edit dialog.** `src/components/AddEditDialog.tsx:103` calls
   `setStack(info.stack)` inside `handleBrowse` only. Opening Edit on an
   existing project initialises `stack` from `editing?.stack` (line 62/74) and
   saves that straight back, so Edit → Save on a project with no stack writes no
   stack.

Confirmed live: the maintainer's registry contains one project (`IELTS Coach`,
added by hand-editing `projects.json`) and it has no `stack` key at all.

Plan 023's own text claimed the stack was "refreshed on Add, on Edit, and during
the install phase". Two of those three do not fire in the common case. This plan
fixes that.

## Current state

`src-tauri/src/run.rs` — the only writer, reached only on a successful install:

```rust
// line ~1046
/// should follow it). Plan 023: also refreshes `stack` in this same save — a successful install is
async fn store_lockfile_hash(app: &AppHandle, project: &Project, hash: &str) {
    let stack = registry::read_package_json(Path::new(&project.path)).stack;
    // ... sets p.last_lockfile_hash and p.stack in one save
}
```

```rust
// line ~1243 — the only call site
PhaseOutcome::Exited(Some(0)) => store_lockfile_hash(app, project, hash).await,
```

`src/components/AddEditDialog.tsx`:

```tsx
const [stack, setStack] = useState<ProjectStack | undefined>(editing?.stack);   // line ~62
// ...
setStack(editing?.stack);                                                       // line ~74 (on open)
// ...
setStack(info.stack);                                                           // line ~103 (handleBrowse ONLY)
```

`registry::read_package_json(dir: &Path) -> PackageJsonInfo` is safe to call on
any path: a missing or unparseable `package.json` yields an empty stack rather
than an error (SPEC.md §10 step 6 — projects with no `package.json` are legal).

`registry::save_projects` is the existing atomic write. Follow the exact
lock-and-save shape `store_lockfile_hash` already uses — mutate under the
`projects` lock, save, and keep the async mutex off the I/O where the existing
code does.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | 104 pass, 3 ignored |
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run verify`, `npm run build`, `npm run build:app`, or
`npm run test:acceptance` — a 600 s no-output watchdog has killed executor runs
here. Keep every Write/Edit under ~60 lines and commit after each.

## Scope

**In scope**:
- `src-tauri/src/run.rs` (refresh the stack on every Run, not only after an install)
- `src/components/AddEditDialog.tsx` (refresh on open, not only on browse)

**Out of scope** (do NOT touch):
- `get_projects` / `to_view`. The stack stays persisted, not derived. That
  command already stats every project, `loadRegistry()` fires on window focus
  (plan 022), and plan 010 — blocking I/O out from under the async locks — is
  still TODO. Adding N `package.json` reads there would multiply a known,
  unfixed problem. This is settled; do not re-open it.
- `registry.rs`'s `detect_stack` or the allow-lists — detection is correct, only
  its trigger points are wrong.
- The install decision, the lockfile hash, the §6 state machine, kill paths.
- Any new §7 command, any new dependency, any schema change.
- A startup migration that rewrites every project. A Run and an Edit are the
  two moments the user is already asking Hangar to look at a project; a
  background rewrite of the registry at launch is a bigger, riskier idea and is
  not needed to fix this.

## Git workflow

- One commit per file: `Stack backfill: <what>`.

## Steps

### Step 1: Refresh the stack on every Run

In `src-tauri/src/run.rs`'s run sequence, refresh `p.stack` at a point that is
reached on **every** Run that gets as far as spawning — not only when an install
happened.

Guidance:
- The natural place is beside where `lastRunAt` is persisted when entering
  `starting` (§5: "set when entering `starting`"), because that save already
  happens on every Run and already holds the record.
- Fold the stack into that **existing** save. Do not add a second write to
  `projects.json` per Run.
- `read_package_json` does filesystem I/O — call it **before** taking the
  `projects` lock and pass the result in, matching how `store_lockfile_hash`
  reads first and mutates second. Do not hold the async mutex across the read.
- Leave `store_lockfile_hash`'s own stack refresh alone. It is harmless (the
  install path re-reads a file it just changed, which is if anything more
  current) and removing it would be a behaviour change this plan does not need.

**Verify**: `cargo check` → exit 0; `cargo test` → 104 pass. Then
`grep -n "read_package_json" src-tauri/src/run.rs` shows two call sites (the
install one and the new one) — record both line numbers in your report.

### Step 2: Refresh in the Edit dialog on open

In `src/components/AddEditDialog.tsx`, when the dialog opens in **edit** mode
for a project whose path is set, call `readPackageJson(path)` and
`setStack(info.stack)` — so opening Edit on an existing project picks up its
stack without the user having to re-browse to the same folder.

Guidance:
- Do it in the existing effect that runs on open (near line ~74, where
  `setStack(editing?.stack)` currently is), not in a new component.
- Seed from `editing?.stack` first so the dialog renders immediately, then
  overwrite when the read resolves — no loading spinner for this.
- Swallow a failed read silently and keep whatever was already there. A project
  whose folder has moved must still be editable; §12's "Project path
  deleted/moved" row says Edit and Remove stay available.
- Guard against setting state after unmount (the file's existing effects already
  use a `cancelled` flag — follow that pattern).

**Verify**: `npm run typecheck` → exit 0.

### Step 3: Gates and commit

**Verify**: `cargo test` → 104 pass, 3 ignored; `npm run typecheck` → exit 0;
`git status --short` shows only the two in-scope files.

## Test plan

No new automated tests: `detect_stack` is already covered by plan 023's six
unit tests, and this plan changes *when* it is called, not what it computes.
Adding a test for the call site would require an `AppHandle`, which nothing in
this codebase constructs outside a running app — the same constraint documented
in plan 020's notes.

Manual checks for the reviewer/maintainer (a subagent cannot drive the GUI):
- With a project that has **no** `stack` in `projects.json`: click Run → after
  it starts, the card shows a framework badge. Confirm `projects.json` now
  contains a `stack` object for it.
- Open Edit on that same project **without** re-browsing → the libraries line
  appears beneath the path.
- A project whose folder was deleted still opens in Edit without error.
- Run a project twice in a row with no dependency change → still exactly one
  `projects.json` write per Run (no double-save regression).

## Done criteria

- [ ] `cargo test` → 104 passed, 3 ignored; `npm run typecheck` → exit 0
- [ ] `grep -n "read_package_json" src-tauri/src/run.rs` shows two call sites (report both)
- [ ] The Run-path refresh reuses the existing `lastRunAt` save — `grep -c "save_projects" src-tauri/src/run.rs` is unchanged from before this plan (report before/after)
- [ ] No change to `get_projects`/`to_view`, no new command, no new dependency, no schema change
- [ ] `plans/README.md` status row for 025 updated

## STOP conditions

Stop and report back if:

- Folding the stack into the `lastRunAt` save would require restructuring that
  save or adding a second write per Run — report the constraint instead.
- You find yourself calling `read_package_json` while holding the `projects`
  lock. Read first, then lock, as `store_lockfile_hash` does.
- The only way to make existing projects show a badge appears to be deriving the
  stack in `get_projects`. It is not, and that path is explicitly out of scope
  for the performance reason given above.

## Maintenance notes

- The lesson worth keeping: plan 023 listed three refresh triggers and two of
  them did not fire in the common case. When a feature's data is written only on
  conditional paths, check what happens to records that predate it — "works for
  new records" is not "works".
- After this, the stack refreshes on: Add (dialog), Edit (open), every Run, and
  the install phase. That is enough; a startup migration remains unnecessary.
- If a project's `package.json` changes while Hangar is open and the project is
  not run, its badge stays stale until the next Run. `detectedAt` is in the Edit
  dialog precisely so that is visible rather than mysterious.
