# Plan 012: Give settings.json the same corruption rescue as projects.json, and harden atomic_write's failure paths

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 91be38f..HEAD -- src-tauri/src/registry.rs src-tauri/src/main.rs src-tauri/src/commands.rs`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: plans/010-lock-hygiene-blocking-io.md (both touch save paths; land 010 first to avoid churn)
- **Category**: bug
- **Planned at**: commit `91be38f`, 2026-08-06

## Why this matters

SPEC.md §4 gives `projects.json` a strict corruption contract — never
overwrite, rename to `.broken-<timestamp>`, surface a banner — and says "Same
rules for `settings.json`". The settings half was never built: a corrupt
`settings.json` silently falls back to defaults, and the next `set_settings`
**overwrites the original bytes**. The load function's own doc comment claims
the file is "left untouched", which is true only until the first save — the
comment documents an intention the code doesn't keep.

Separately, `atomic_write` (used for both files) has two failure-path gaps:
a failed write leaves a stale `<name>.tmp` next to the registry forever, and
the rename is never made durable with a parent-directory fsync — so the
module's implied guarantee is stronger than what the code provides.

## Current state

`src-tauri/src/registry.rs`:

- `atomic_write` (~line 122-147):

```rust
pub fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    ...
    let tmp = dir.join(format!("{file_name}.tmp"));

    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }

    // rename is atomic on both macOS/Linux and Windows for same-directory targets.
    std::fs::rename(&tmp, path)
}
```

Early `?` returns after the tmp exists leave it on disk; no directory fsync
after the rename.

- `load_settings` (~line 244-256):

```rust
/// Settings load. A missing file is created with the default. A present-but-unparseable file is
/// left untouched and the default is used — the next `set_settings` rewrites it atomically.
pub fn load_settings(dir: &Path) -> Settings {
    ...
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}
```

- The `projects.json` recovery pattern to copy (same file, `load_projects`,
  ~line 170-230): on parse failure it renames to
  `projects.json.broken-<unix-timestamp>` (helper `unix_timestamp()`, ~line
  149) and returns a `RegistryError { backup_path, error }` that
  `main.rs::setup` stores in `AppState.registry_error` for the §11 banner.
- `RegistryError` (~line 87) is projects-specific in name only; its banner
  channel (`get_registry_error` command) currently carries at most one error.
- Existing tests: `corrupt_file_is_renamed_not_overwritten_and_bytes_survive`,
  `atomic_write_leaves_no_temp_file_and_replaces_content`,
  `settings_default_and_round_trip` — the structural patterns.

Constraint from SPEC.md §4: do NOT add `tauri-plugin-fs`/`tauri-plugin-store`;
plain `std::fs` in this module is the design.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass |
| Windows check | `PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc` | exit 0 |
| TypeScript / build | `npx tsc --noEmit && npm run build` | exit 0 |

## Scope

**In scope**:
- `src-tauri/src/registry.rs`

**Out of scope** (do NOT touch):
- `src-tauri/src/main.rs`, `src-tauri/src/commands.rs`, the frontend banner —
  surfacing the settings backup in the UI needs a second banner slot and §7
  thought; this plan LOGS the rescue and preserves the bytes, which is the
  data-loss half. UI surfacing is deferred (see Maintenance notes).
- `save_projects`/`save_settings` call sites in other files (plan 010 owns the
  locking around them).

## Git workflow

- Work on `main`. One commit:
  `Rescue corrupt settings.json like projects.json and harden atomic_write failure paths`

## Steps

### Step 1: settings.json rescue

Reshape `load_settings` to mirror `load_projects`' corruption arm:

- Parse failure → rename the file to `settings.json.broken-<unix_timestamp()>`
  (reuse the existing helper and naming convention), `eprintln!` a
  `hangar:`-prefixed line naming the backup path and the parse error (matching
  the module's existing eprintln style), and return `Settings::default()`.
- Read failure (`Err(_)` on `std::fs::read`) → keep returning defaults but
  eprintln the error too (today it is fully silent).
- Fix the doc comment to describe the new behaviour.
- Rename-failure arm: if the rescue rename itself fails, do NOT write defaults
  over the file — return defaults in memory and eprintln that the file was
  left in place unrescued. (This preserves "never overwrite" even when the
  rescue fails; the next `set_settings` MAY still overwrite — acceptable and
  noted in the comment, since blocking saves on a failed rename would brick
  Settings entirely.)

**Verify**: `cargo test` → existing settings tests pass.

### Step 2: atomic_write — unique tmp, cleanup on failure, dir fsync

- Tmp name: `format!("{file_name}.tmp.{}", std::process::id())` plus a
  process-local `AtomicU64` counter suffix — unique across concurrent writers
  and across a crashed predecessor's leftovers.
- Failure cleanup: on any error after the tmp is created (write, sync, rename),
  best-effort `let _ = std::fs::remove_file(&tmp);` before returning the error.
- Durability: after a successful rename, on Unix only, open the parent
  directory and `sync_all()` it (`#[cfg(unix)]`, best-effort — ignore its
  error with a comment: the rename's atomicity is the §4 guarantee; the dir
  fsync upgrades power-loss durability from old-or-new to definitely-new).
  On Windows, directory handles don't support this — comment that and skip.
- Startup hygiene: in `load_projects` (which already runs once at startup),
  best-effort remove any `projects.json.tmp*` / stale `*.tmp.*` siblings left
  by a crashed writer — one `read_dir` scan of the config dir, filenames
  starting with `PROJECTS_FILE` or `SETTINGS_FILE` and containing `.tmp`.

**Verify**: `cargo test` → `atomic_write_leaves_no_temp_file_and_replaces_content`
still passes.

### Step 3: Tests

Following the existing test style in registry.rs:

1. `corrupt_settings_is_renamed_not_overwritten` — write invalid JSON to
   `settings.json` in a temp dir, `load_settings`, assert: return equals
   defaults, original bytes exist in exactly one `settings.json.broken-*`
   file, and `settings.json` itself no longer exists (rescued, not copied).
2. `atomic_write_cleans_up_its_temp_file_when_the_rename_target_is_invalid` —
   force a failure after tmp creation (e.g. target path whose parent is a
   FILE, or make the target a directory so rename fails), assert the error
   returns AND no `*.tmp*` file remains in the temp dir.
3. `stale_temp_files_are_swept_at_startup` — drop a fake
   `projects.json.tmp.999.1` in the dir, run `load_projects`, assert it is
   gone and the load result is unaffected.

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass,
3 new tests.

### Step 4: All gates, then commit

**Verify**: four gate commands green; `git status` shows only registry.rs.

## Test plan

The three tests above; plus every existing registry test unmodified — the
projects.json path must be behaviour-identical except for the startup sweep.

## Done criteria

- [ ] All gates green; 3 new tests pass; existing tests unmodified
- [ ] `grep -n "left untouched" src-tauri/src/registry.rs` → no stale claim (comment updated)
- [ ] Only `src-tauri/src/registry.rs` modified
- [ ] `plans/README.md` status row for 012 updated

## STOP conditions

Stop and report back if:

- The excerpts don't match (drift — especially if plan 010 moved call sites).
- Making the settings rescue work appears to require touching `main.rs` or the
  §7 API (it must not — the UI banner is explicitly deferred).
- The Unix dir-fsync causes a test/permission failure in the sandboxed test
  temp dir after one fix attempt — downgrade to best-effort-with-comment and
  note it in the report.

## Maintenance notes

- Deferred deliberately: surfacing the settings backup in the §11 banner —
  needs either a second `RegistryError` slot or a list, which touches the §7
  deviation (`get_registry_error`). Fold into plan 005's Settings dialog work
  or a later run.
- Plan 005 adds Add/Edit/Remove writers; they inherit this hardened
  `atomic_write` for free but its reviewer should confirm no new direct
  `std::fs::write` sneaks in beside it.
