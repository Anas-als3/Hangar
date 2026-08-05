# Plan 001: Scaffold Hangar with working storage and an app shell that renders projects

> **Executor instructions**: Follow this plan step by step. Run every verification command
> and confirm the expected result before moving to the next step. If anything in the "STOP
> conditions" section occurs, stop and report — do not improvise. When done, update the
> status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `git log --oneline` — you should see `e74666e` as the only
> commit (or as an ancestor). `ls src-tauri` should fail (nothing scaffolded yet). If a
> scaffold already exists, STOP and report.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED (scaffold choices are load-bearing for every later plan)
- **Depends on**: none
- **Category**: migration (greenfield scaffold)
- **Planned at**: commit `e74666e`, 2026-08-05

## Why this matters

Every later milestone builds directly on the choices made here: the Tauri 2 ACL capability
file, the bundle identifier, the plugin registration order, the storage layer, and the file
layout. Getting the scaffold wrong is not a cosmetic problem — a missing capability entry
produces a runtime permission denial with no compile-time signal, and a `std::sync::Mutex`
in managed state produces a compile error in plan 002 that an executor will "fix" by making
the process manager synchronous, which then makes the kill sequence in plan 003 impossible.
This plan ends with an app that reads and writes its registry safely and renders it.

## Current state

The repo contains only planning artifacts:

- `SPEC.md` — the full specification. **Read §4 (stack & platform rules), §5 (data model &
  storage), §7 (frozen command/event API), §11 (UI direction), §13 (repository layout)
  before writing code.** They contain the exact config snippets, palette hex values, and
  file layout this plan refers to and does not repeat.
- `CLAUDE.md` — standing rules for every milestone.
- `plans/README.md` — verification gates and environment facts.

No `package.json`, no `src-tauri/`, no source of any kind.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust (host) | `cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| Rust (Windows) | `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc` | exit 0 (best-effort — see plans/README.md) |
| Rust tests | `cargo test --manifest-path src-tauri/Cargo.toml` | all pass |
| TypeScript | `npx tsc --noEmit` | exit 0 |
| Frontend build | `npm run build` | exit 0 |

The first `cargo check` after scaffolding compiles the entire Tauri dependency tree and may
take 5–15 minutes. This is normal — do not interrupt it or conclude it has hung.

## Scope

**In scope** (create):
- `package.json`, `vite.config.ts`, `tsconfig.json`, `index.html`, `src/**`
- `src-tauri/**` (`Cargo.toml`, `tauri.conf.json`, `build.rs`, `src/**`, `capabilities/**`, `icons/**`)

**Out of scope** (do NOT touch):
- `SPEC.md`, `CLAUDE.md` — the specification is fixed input, not something to edit.
- `plans/001-*.md` … `plans/006-*.md` — only `plans/README.md`'s status row.
- Any process spawning, killing, git, install, or port logic — that is plans 002–006.
  Create the module files as stubs only where §13 requires them to exist.

## Git workflow

- Work directly on the current branch (this is a greenfield repo; no branching needed).
- One commit at the end of the plan, message style matching the baseline commit:
  `Add M1 scaffold: Tauri 2 + React + storage layer` with a short body.
- Do NOT push (there is no remote).

## Steps

### Step 1: Scaffold the Tauri 2 + React + TypeScript + Vite project

Scaffold into the **current directory** (which already contains `SPEC.md`, `CLAUDE.md`,
`plans/`, `.gitignore`, `.git/`). `create-tauri-app` refuses to write into a non-empty
directory in some versions — if that happens, scaffold into a temporary subdirectory and
move the generated files up, then delete the empty subdirectory. Do not delete or overwrite
the existing files.

Use: React, TypeScript, npm. Then `npm install`.

Set in `src-tauri/tauri.conf.json`: `identifier` = `com.hangar.app`, window title `Hangar`,
and a sensible default window size (1100×720 or larger — the grid needs room).

**Verify**: `npm run build` → exit 0, and `ls src-tauri/src/main.rs` → file exists.

### Step 2: Add Tailwind v4 and the bundled fonts

Per SPEC.md §4: Tailwind v4 through the `@tailwindcss/vite` plugin with a single
`@import "tailwindcss";` in the entry stylesheet. **No `tailwind.config.js`, no PostCSS
config** — v4 does not need them.

Add the three Fontsource packages named in §4 and import them in the entry stylesheet.
Define the §11 palette and the three font families as CSS custom properties / Tailwind v4
`@theme` tokens so later plans reference tokens, never raw hex.

Confirm `tauri.conf.json`'s `build.devUrl` matches Vite's dev port and `frontendDist` points
at Vite's build output directory.

**Verify**: `npm run build` → exit 0; `grep -r "tailwindcss" src/*.css` → matches the import;
`ls node_modules/@fontsource/space-grotesk` → exists.

### Step 3: Register plugins and the ACL capability file

Add Cargo dependencies `tauri-plugin-dialog = "2"`, `tauri-plugin-opener = "2"`,
`tauri-plugin-single-instance = "2"` and npm package `@tauri-apps/plugin-dialog`.

Create `src-tauri/capabilities/default.json` exactly as given in SPEC.md §4.

In `main.rs`, register **single-instance first** (per §4), then dialog and opener. The
single-instance callback focuses the existing main window.

Do **not** add `tauri-plugin-shell` or `tauri-plugin-fs` or `tauri-plugin-store`.

**Verify**: `cargo check --manifest-path src-tauri/Cargo.toml` → exit 0;
`grep -c "tauri-plugin-shell" src-tauri/Cargo.toml` → 0 matches.

### Step 4: Create the module layout from SPEC.md §13

Create every Rust module and frontend file named in §13. Modules that later plans fill
(`process.rs`, `run.rs`, `env_resolve.rs`) get a file with a doc comment naming the plan
that implements them and nothing else — do not stub fake functions that later plans must
delete.

`src/types.ts` is the single source of truth mirroring the Rust types: `Status` (all eight
variants from §5), `Project`, `ProjectView`, `LogLine`, and the two event payload types
from §7. Rust structs derive `Serialize`/`Deserialize` with
`#[serde(rename_all = "camelCase")]` so the two sides match exactly.

**Verify**: `npx tsc --noEmit` → exit 0; every path listed in §13 exists (`ls` each).

### Step 5: Implement the storage layer in `registry.rs`

Implement per SPEC.md §4 (Storage) and §5:

- Path from `app.path().app_config_dir()`, `create_dir_all` before first write.
- **Atomic writes**: serialize → write `<name>.tmp` in the same directory → `rename` over
  the target. Never write the target file directly.
- Startup load: file absent → write `[]`. File present but unparseable → **never
  overwrite**; rename to `projects.json.broken-<unix-timestamp>`, return an empty registry
  plus a flag the UI surfaces as a persistent banner naming the backup file and the parse
  error. Unknown fields ignored, never fatal.
- `settings.json` holding `{ "editorCommand": "code" }`, same atomic rules.
- `HANGAR_DEV_SEED` env var: when set **and** `projects.json` is absent, write the §5 seed
  entry instead of `[]`.

Add `#[cfg(test)]` unit tests in this module for the pure logic: round-trip
serialize/deserialize of `Project`, the corrupt-file rename path, and that unknown JSON
fields do not fail parsing. Use `tempfile`-free tests (build paths under
`std::env::temp_dir()`) so no new dependency is needed.

**Verify**: `cargo test --manifest-path src-tauri/Cargo.toml` → all pass, at least 3 tests run.

### Step 6: Wire the read-only slice of the frozen command API

Implement **only** these commands from SPEC.md §7 (the rest belong to later plans):
`get_projects`, `get_settings`, `set_settings`. Signatures and names exactly as §7 gives
them — the API is frozen.

Managed state per §4: `tokio::sync::Mutex` inside `app.manage(...)`, commands are `async`.
Even though nothing awaits yet, using `std::sync::Mutex` here forces a rewrite in plan 002.

`src/api.ts` holds every `invoke()` call; no component calls `invoke` directly.

**Verify**: `cargo check --manifest-path src-tauri/Cargo.toml` → exit 0;
`grep -rn "invoke(" src/ --include=*.tsx` → no matches (all invokes live in `src/api.ts`).

### Step 7: Build the app shell UI

Per SPEC.md §11. Scope for this plan: the grid, the card, the empty state, the corrupt-file
banner. Buttons render and are wired to nothing yet — that is correct at M1.

- `ProjectGrid` renders cards in `projects.json` array order (§11 — no sorting, ever).
- `ProjectCard` shows name (Space Grotesk), status pill with port in mono, the time slot,
  a primary button, and an overflow menu (items may be disabled at this milestone).
- Empty state exactly per §11, with an Add button (opens nothing yet).
- `pathExists: false` renders the warning state with Run disabled.
- Respect `prefers-reduced-motion`. Palette via the tokens from step 2 — no raw hex in
  components.

**Verify**: `npm run build` → exit 0; `npx tsc --noEmit` → exit 0.

### Step 8: Run all gates and commit

Run all five gates from `plans/README.md`. Then commit.

**Verify**: `git status --short` → clean tree after commit.

## Test plan

- Rust unit tests in `registry.rs` (step 5): `Project` serde round-trip; corrupt file is
  renamed rather than overwritten and the original bytes survive in the `.broken-*` file;
  unknown JSON fields parse without error.
- No JS test runner is added at this milestone (see `plans/README.md` rejected findings).
- Manual check the reviewer will perform: `npm run tauri dev` opens a window showing the
  empty state; `HANGAR_DEV_SEED=1 npm run tauri dev` shows one card in warning state
  (the seed's path is the literal placeholder, so `pathExists` is false — this is expected
  and correct per §5).

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo check --manifest-path src-tauri/Cargo.toml` exits 0
- [ ] `cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc` exits 0, or fails only outside `src-tauri/src/` with the error recorded in the report
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` exits 0 with ≥3 tests passing
- [ ] `npx tsc --noEmit` exits 0
- [ ] `npm run build` exits 0
- [ ] `src-tauri/capabilities/default.json` exists and lists `core:default`, `dialog:default`, `opener:default`
- [ ] `grep -c "tauri-plugin-shell" src-tauri/Cargo.toml` returns 0
- [ ] `grep -rn "std::sync::Mutex" src-tauri/src/` returns no matches
- [ ] Every file path in SPEC.md §13 exists
- [ ] `git status --short` is clean; exactly one new commit
- [ ] `plans/README.md` status row for 001 updated

## STOP conditions

Stop and report back (do not improvise) if:

- A scaffold already exists in the repo (drift — someone ran M1 before you).
- `create-tauri-app` produces a Tauri **1.x** project (check `tauri.conf.json` shape and the
  `tauri` crate major version). This plan is Tauri 2 only; do not attempt to migrate.
- Tailwind v4's `@tailwindcss/vite` plugin does not exist or fails to build. Do **not**
  silently fall back to Tailwind v3 with a config file — report and stop.
- The first `cargo check` fails with linker errors mentioning missing Xcode components.
- A SPEC.md snippet does not match the current API of the installed Tauri version. Per
  `CLAUDE.md`, keep the spec's **intent**, follow the compiler/current docs, add a code
  comment noting the deviation — and list every such deviation in your report.

## Maintenance notes

- Plan 002 replaces the stub `process.rs` and `env_resolve.rs`; it depends on managed state
  already using `tokio::sync::Mutex` and on `src/types.ts` matching the Rust structs.
- A reviewer should scrutinize: the capability file's permission list, that atomic write
  really is temp+rename (not truncate-in-place), and that the corrupt-file path cannot
  destroy user data.
- Deferred deliberately: any process, port, git, or install behavior; wiring the Add dialog;
  the phase strip (plan 006).
