# Plan 015: Gate the Unix-only acceptance test behind `cfg(unix)` and make the Windows gate typecheck test code

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat 3fdb47e..HEAD -- src-tauri/src/process.rs plans/README.md .github/workflows/ci.yml`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P1 — `main` is currently RED in CI
- **Effort**: S
- **Risk**: LOW
- **Depends on**: plans/009-ci-and-verify-entrypoint.md (DONE — it is what surfaced this)
- **Category**: bug
- **Planned at**: commit `3fdb47e`, 2026-08-06

## Why this matters

The first CI run after plan 009 landed failed its `windows` job. The Rust
*library* compiles fine on Windows; the **test binary** does not:

```
error[E0425]: cannot find function `count_node_processes` in this scope
  --> src\process.rs:1975:26
error[E0425]: cannot find function `list_group` in this scope
  --> src\process.rs:2006:69
error: could not compile `hangar` (bin "hangar" test) due to 4 previous errors
```

`the_orphan_test_leaves_no_node_processes_behind` (SPEC.md §15 test 3) is
`#[test] #[ignore]` but **not** `#[cfg(unix)]`, while the two helpers it calls
(`count_node_processes`, `list_group`) *are* `#[cfg(unix)]`. On Windows the test
body is compiled and the helpers do not exist. The two equivalent acceptance
tests in `run.rs` are already correctly gated — this one is the odd one out.

Two things follow, and the second is the important one:

1. `main` is red, and the `windows` job cannot run **any** unit test until the
   test binary builds. The whole reason that job exists is to give the
   `#[cfg(windows)]` kill path (Job Object creation and assignment,
   `TerminateJobObject` via a hand-declared `extern "system"` block, the
   `taskkill` fallback) its first-ever *execution*. Right now it still has
   never executed.
2. This hid for three milestones because the local Windows gate is
   `cargo check --target x86_64-pc-windows-msvc`, which does **not** typecheck
   test code. Verified on 2026-08-06: that command reports **0 errors**, while
   the same command with `--all-targets` reproduces all four CI errors in
   seconds. The gate has been blind to every `#[cfg]` mistake in test code
   since it was introduced.

## Current state

`src-tauri/src/process.rs`, the failing test (~line 1939) — note the missing
`#[cfg(unix)]`:

```rust
    #[test]
    #[ignore]
    fn the_orphan_test_leaves_no_node_processes_behind() {
```

Its helpers, correctly gated (~lines 2045 and 2057):

```rust
    #[cfg(unix)]
    async fn list_group(pgid: u32) -> String {
    ...
    #[cfg(unix)]
    async fn count_node_processes() -> usize {
```

The correct pattern already in the repo — `src-tauri/src/run.rs` (~lines 1512
and 1603), both gated:

```rust
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn a_ready_timeout_kills_the_tree_and_leaves_no_orphans() {
    ...
    #[test]
    #[ignore]
    #[cfg(unix)]
    fn a_command_that_exits_at_once_is_diagnosed_at_once_not_after_the_timeout() {
```

The gate table in `plans/README.md` (§ "Verification gates") has a
"Rust (Windows paths)" row whose raw command is:

```
PATH="/opt/homebrew/opt/llvm/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc
```

`.github/workflows/ci.yml` has three jobs (`host`, `acceptance`, `windows`);
the `windows` job runs `npm run build` then
`cargo test --manifest-path src-tauri/Cargo.toml`.

Conventions: comments cite the SPEC section they implement. The `#[ignore]`
acceptance tests are documented in `run.rs`'s test module as requiring
`--test-threads=1` because they count `node` processes machine-wide.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Reproduce the bug | `PATH="$HOME/.cargo/bin:/opt/homebrew/opt/llvm/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --all-targets` | 4 × E0425 **before** the fix, exit 0 **after** |
| Full local verify | `npm run verify` | exit 0 |
| Acceptance lane (Unix) | `npm run test:acceptance` | 3 pass |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | 76 pass, 3 ignored |

## Scope

**In scope**:
- `src-tauri/src/process.rs` (the one test attribute)
- `plans/README.md` (the Windows gate row's command only)
- `.github/workflows/ci.yml` (add the `--all-targets` check to the `windows` job)

**Out of scope** (do NOT touch):
- The helpers `count_node_processes` / `list_group` — do NOT make them
  cross-platform. They shell out to `pgrep`, which is the SPEC.md §15
  measurement; a Windows equivalent is a separate, larger decision.
- Any production (non-test) Rust code.
- The `host` / `acceptance` CI jobs — they pass.
- The other two acceptance tests in `run.rs` — already correct.

## Git workflow

- One commit: `Gate the Unix-only orphan test and make the Windows gate typecheck test code`

## Steps

### Step 1: Reproduce the failure locally

Run the "Reproduce the bug" command from the table above **before changing
anything**. Confirm you see 4 × `E0425` naming `count_node_processes` and
`list_group`. Record the output in your report — this is what proves the fix.

**Verify**: 4 errors, exit non-zero.

### Step 2: Gate the test

Add `#[cfg(unix)]` to `the_orphan_test_leaves_no_node_processes_behind` in
`src-tauri/src/process.rs`, matching the attribute order used by the two
already-correct tests in `run.rs` (`#[test]`, `#[ignore]`, `#[cfg(unix)]`).

Add a brief comment above it explaining WHY, in the repo's style — something
like: the fixture and its measurement shell out to `pgrep`/`lsof`, which are
the SPEC.md §15 measurement and have no Windows equivalent here; the Windows
kill path's own coverage is the `#[cfg(windows)]` unit tests, which this gating
is what allows to compile and run at all.

**Verify**: the step 1 command now exits 0 with no errors.

### Step 3: Make the local Windows gate see test code

In `plans/README.md`'s gate table, change the "Rust (Windows paths)" raw
command to append `--all-targets`, and extend the existing Windows-target note
to say why: without it the gate typechecks only the library and is blind to
`#[cfg]` errors in test code — which is exactly how this bug survived three
milestones and was first caught by CI rather than locally.

**Verify**: `grep -n -- "--all-targets" plans/README.md` → at least 1 match.

### Step 4: Make CI catch it at the cheap stage too

In `.github/workflows/ci.yml`'s `windows` job, add a step that runs
`cargo check --manifest-path src-tauri/Cargo.toml --all-targets` **before** the
existing `cargo test` step, so a future `#[cfg]` mistake fails in seconds with
a clear message rather than midway through a full test build. Keep the existing
`cargo test` step — it is what actually executes the Windows kill path.

**Verify**: the YAML still parses —
`python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`
exits 0 (install `pyyaml` locally if missing; do NOT add it to any project
manifest).

### Step 5: Make the `windows` job informative, not blocking

**Maintainer decision, 2026-08-06**: macOS is the development and dogfooding
platform (SPEC.md §15 test 9 is a two-week macOS usage trial). Windows
correctness matters for v1 but must not gate day-to-day Mac work — and step 2
is about to let the Windows unit tests EXECUTE for the first time ever, which
may well surface a genuine failure in the never-run Job Object code.

In `.github/workflows/ci.yml`, add `continue-on-error: true` to the `windows`
job (job level, not step level), with a comment recording the decision and its
reversal condition:

```yaml
  windows:
    runs-on: windows-latest
    # Informative, not blocking: macOS is the current development platform
    # (SPEC.md §15 test 9). This job still compiles and RUNS the #[cfg(windows)]
    # kill path — Job Objects, TerminateJobObject, the taskkill fallback — and a
    # red result here is a real finding worth a plan. Flip this back to blocking
    # when Windows becomes a supported target in earnest.
    continue-on-error: true
```

Leave `host` and `acceptance` blocking — they cover the platform actually in
use, and they are green.

**Verify**: `python3 -c "import yaml; d=yaml.safe_load(open('.github/workflows/ci.yml')); print(d['jobs']['windows'].get('continue-on-error'), d['jobs']['host'].get('continue-on-error'), d['jobs']['acceptance'].get('continue-on-error'))"`
→ prints `True None None`.

### Step 6: Full local gates, then commit

Run `npm run verify` (exit 0), `npm run test:acceptance` (3 pass — proves the
gating did not disable the Unix tests), and the step 1 command (exit 0).

**Verify**: all three green; `git status --short` shows only the three in-scope
files.

## Test plan

No new tests. The verification IS the reproduction: the step 1 command must go
from 4 errors to exit 0, and `npm run test:acceptance` must still run 3 tests on
macOS — proving the `#[cfg(unix)]` gate did not accidentally disable them here.

## Done criteria

- [ ] `cargo check --target x86_64-pc-windows-msvc --all-targets` exits 0 (was 4 errors — both recorded in the report)
- [ ] `npm run verify` exits 0
- [ ] `npm run test:acceptance` → 3 passed (the Unix tests still run)
- [ ] `grep -n "cfg(unix)" src-tauri/src/process.rs` includes the orphan test
- [ ] `plans/README.md` Windows gate row contains `--all-targets`
- [ ] `.github/workflows/ci.yml` parses and the `windows` job has an `--all-targets` check before `cargo test`
- [ ] The `windows` job is `continue-on-error: true`; `host` and `acceptance` are NOT
- [ ] No production Rust code modified

## STOP conditions

Stop and report back if:

- The step 1 command does NOT reproduce the 4 errors (drift — the fix may
  already be in).
- Adding `#[cfg(unix)]` makes `npm run test:acceptance` run fewer than 3 tests
  on macOS — that means the gate is wrong, not the test.
- `cargo check --all-targets` for the Windows target surfaces errors in
  **production** `#[cfg(windows)]` code (not just test code) — that is a
  genuinely new finding about the kill path; report it in full and do not try
  to fix it under this plan.

## Maintenance notes

- After this lands, the `windows` CI job will attempt to RUN the Windows unit
  tests for the first time. Expect the possibility of a second, more
  interesting failure — in the Job Object / `TerminateJobObject` code that has
  never executed. That is a separate finding and deserves its own plan.
- Any future `#[ignore]` acceptance test that shells out to Unix tools must
  carry `#[cfg(unix)]`. The `--all-targets` gate now enforces this locally.
- Reviewer should scrutinize: the acceptance lane still reports 3 passing tests
  on macOS (the gate must not silently reduce Unix coverage).
