# Plan 009: One `verify` entrypoint for the gates, CI that runs them (including Windows), and a de-trapped `dev` script

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 91be38f..HEAD -- package.json src-tauri/tauri.conf.json plans/README.md`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none (008 lands more tests the CI will then run — order 008 first if possible, not required)
- **Category**: dx
- **Planned at**: commit `91be38f`, 2026-08-06

## Why this matters

This repo's five verification gates exist only as prose in `plans/README.md`,
runnable by pasting long incantations (one of which silently verifies nothing
without a PATH prefix). There is no CI at all — no `.github/` directory — so
every gate depends on an executor remembering to run it and reporting honestly.
The cost is not hypothetical: commit `aa46b6d` (M4) silently reverted commit
`5cb7c2b`'s race fix and had to be restored by `91be38f` a few hours later; CI
running `cargo test` on every push would have flagged it immediately. A CI
Windows job also gives the Windows kill path (Job Objects) its first-ever
*execution*, not just typecheck — `plans/README.md` currently defers that to "a
human on Windows" who has never materialized.

Separately, `npm run dev` is a trap: it starts bare Vite in a browser where
every `invoke()` rejects, rendering what looks like a broken app. The correct
command (`npm run tauri dev`) appears in exactly one place in the repo, deep in
a plan file.

## Current state

- `package.json` scripts (as of `91be38f`):

```json
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview",
    "tauri": "tauri"
  }
```

- `src-tauri/tauri.conf.json` `build.beforeDevCommand` is `"npm run dev"` and
  `beforeBuildCommand` is `"npm run build"` — renaming the `dev` script MUST
  update `beforeDevCommand` in the same commit or `tauri dev` recurses/breaks.
- The gate list (5 rows) lives in `plans/README.md` (~lines 29-39): host
  `cargo check`, Windows-target `cargo check` (requires
  `PATH="/opt/homebrew/opt/llvm/bin:$PATH"` for `llvm-rc` — documented there in
  detail), `cargo test`, `npx tsc --noEmit`, `npm run build`.
- Three `#[ignore]`d acceptance tests (SPEC §15 tests 3, 7, 4-bonus) run only
  via `cargo test ... -- --ignored --nocapture --test-threads=1` — the
  `--test-threads=1` is REQUIRED because the tests count `node` processes
  machine-wide (documented in `src-tauri/src/run.rs` test-module comments,
  ~line 1389).
- No `.github/`, no CI, no advisory audit anywhere.
- Environment facts (from `plans/README.md`): macOS arm64 dev machine, Node
  v24, Rust 1.97; `cargo` in `~/.cargo/bin` (NOT on default PATH in
  non-login shells).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Full local verify (after this plan) | `npm run verify` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass |
| TypeScript | `npx tsc --noEmit` | exit 0 |
| Frontend build | `npm run build` | exit 0 |
| Workflow lint (if available) | `gh workflow list` after push, or `actionlint` if installed | no errors |

## Scope

**In scope** (the only files you should modify/create):
- `package.json` (scripts only)
- `src-tauri/tauri.conf.json` (`build.beforeDevCommand` only)
- `.github/workflows/ci.yml` (create)
- `plans/README.md` (gate table: point it at the new scripts; status row)

**Out of scope** (do NOT touch):
- Any Rust or TS source file. If a gate fails in CI, that is a report, not a
  license to fix source in this plan.
- Dependencies — add NOTHING to `dependencies`/`devDependencies`.
- Branch protection, release workflows, publishing — none of that here.

## Git workflow

- Work on `main`. One commit:
  `Add verify scripts, GitHub Actions CI with a Windows job, and de-trap npm run dev`
- Push is REQUIRED for CI verification (the repo's remote is
  `https://github.com/Anas-als3/Hangar`, private). If you cannot push, leave
  the commit local and record that step 5 is unverified.

## Steps

### Step 1: Scripts — one name per gate, one `verify` for all

Edit `package.json` scripts to:

```json
  "scripts": {
    "dev": "tauri dev",
    "dev:web": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview",
    "tauri": "tauri",
    "typecheck": "tsc --noEmit",
    "check:rust": "cargo check --manifest-path src-tauri/Cargo.toml",
    "test:rust": "cargo test --manifest-path src-tauri/Cargo.toml",
    "test:acceptance": "cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture --test-threads=1",
    "verify": "npm run check:rust && npm run test:rust && npm run typecheck && npm run build"
  }
```

And in `src-tauri/tauri.conf.json` set `"beforeDevCommand": "npm run dev:web"`.
(Leave `beforeBuildCommand` alone — `npm run build` is still correct.)

Notes for the executor:
- `verify` deliberately omits the Windows-target check (macOS-only PATH quirk)
  and the acceptance lane (spawns real processes) — they stay documented rows
  and CI jobs, not part of the everyday loop.
- Scripts assume `cargo` on PATH; document in the gate table that shells
  without it need `PATH="$HOME/.cargo/bin:$PATH"`.

**Verify**:
1. `npm run verify` → exit 0.
2. `npm run dev` now starts the Tauri app (kill it after the window appears).
3. `grep -n "beforeDevCommand" src-tauri/tauri.conf.json` → shows `dev:web`.

### Step 2: The CI workflow

Create `.github/workflows/ci.yml`:

- Trigger: `push` to `main`, `pull_request`.
- **Job `host` (`macos-latest`)**: checkout; `dtolnay/rust-toolchain@stable`;
  `Swatinem/rust-cache@v2` with `workspaces: src-tauri`; `actions/setup-node@v4`
  (node 24, cache npm); `npm ci`; then `npm run verify`.
- **Job `acceptance` (`macos-latest`)**: same setup; `npm ci`;
  `npm run test:acceptance`. Runners have `node` on PATH, and the serial flag
  makes the machine-wide `pgrep` counting safe on a fresh runner.
- **Job `windows` (`windows-latest`)**: checkout; rust toolchain; rust-cache;
  setup-node; `npm ci`; `npm run build` (the Tauri build script needs the
  frontend `dist/` only for `tauri build`, not for `cargo test` — if
  `cargo test` complains about missing `dist`, create it with the build);
  then `cargo test --manifest-path src-tauri/Cargo.toml`. This is the first
  place the `#[cfg(windows)]` kill path (Job Object creation, assignment,
  `TerminateJobObject` extern) actually COMPILES-AND-RUNS its unit tests.
  Do NOT add the `--ignored` acceptance lane on Windows — those tests use
  `pgrep`/Unix fixtures.
- No Linux job for now: the Tauri Linux build needs a webkitgtk apt setup that
  is real work; note it as a deliberate omission in a workflow comment.

**Verify**: workflow file parses — `npx --yes yaml-lint .github/workflows/ci.yml`
exits 0 (or `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"`).

### Step 3: Update the gate table

In `plans/README.md`, rewrite the gate table rows to name the npm scripts as
the canonical invocations (keeping the raw commands as a second column for
CI-less contexts), fix the "All four must exit 0" sentence to "All five", and
add a row for `npm run test:acceptance` marked "serial, spawns real node
processes". Keep the llvm-rc note — it still applies to the local
Windows-target typecheck row.

**Verify**: `grep -n "All five" plans/README.md` → 1 match;
`grep -n "test:acceptance" plans/README.md` → ≥1 match.

### Step 4: Commit and push

Commit (message above). Push to `origin main`.

**Verify**: `git status` clean; `git log --oneline -1` shows the commit pushed.

### Step 5: Watch the first CI run

`gh run watch` (or `gh run list --limit 1` then `gh run view <id>`) until all
three jobs finish.

**Verify**: all three jobs green. If `windows` or `acceptance` fails for
environment reasons (missing tool on the runner, `pgrep` behaviour), capture
the exact failing step's log excerpt in your report and mark the job
`continue-on-error: true` with a `TODO(plan-009)` comment ONLY for that job —
the host job must stay required and green.

## Test plan

CI itself is the test. The meta-checks: `npm run verify` green locally before
push; the first CI run's three jobs green (or documented per-job exceptions per
step 5); `npm run dev` opens the app window.

## Done criteria

- [ ] `npm run verify` exits 0 locally
- [ ] `npm run dev` starts the Tauri app (not bare Vite)
- [ ] `.github/workflows/ci.yml` exists; first run's `host` job green
- [ ] `windows` job ran `cargo test` (green, or documented failure + continue-on-error per step 5)
- [ ] `plans/README.md` gate table names the scripts; "All five" fixed; row for 009 updated
- [ ] No source files modified

## STOP conditions

Stop and report back if:

- `npm run verify` fails on the CURRENT tree before any of your changes —
  the baseline is broken and that finding outranks this plan.
- `tauri dev` no longer launches after the script rename (the
  `beforeDevCommand` coupling bit) and one fix attempt doesn't restore it.
- The Windows CI job fails INSIDE `src-tauri/src/` code (not environment) —
  that is a real Windows bug this machine could never see; report the log.
- Pushing to origin is impossible (auth) — finish locally, mark step 5
  unverified.

## Maintenance notes

- Plans 005/006 add commands and UI; their executors should run `npm run verify`
  and will get CI on push for free — update their plan text's gate references
  only if they fail to resolve.
- The `acceptance` job's machine-wide process counting is the flakiness
  candidate; if it flakes twice, gate it to `workflow_dispatch` instead of
  deleting it.
- Deferred deliberately: a Linux job (webkitgtk setup), `cargo audit` /
  `npm audit` advisory rows (finding DEPS-02 — add as a follow-up row once CI
  is stable), clippy/fmt gates (one cosmetic warning exists today; adding
  `-D warnings` needs a triage pass first).
