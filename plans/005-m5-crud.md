# Plan 005: Add, edit, and remove projects from the UI, with real validation

> **Executor instructions**: Follow this plan step by step. Run every verification command
> and confirm the expected result before moving to the next step. If anything in the "STOP
> conditions" section occurs, stop and report — do not improvise. When done, update the
> status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `plans/README.md` must show 001–004 as DONE, and
> `npm run verify` must exit 0 before you change anything. Also run
> `git diff --stat 2243d40..HEAD -- src-tauri/src/commands.rs src-tauri/src/registry.rs src/api.ts src/store.ts src/App.tsx src/components/`
> — on any change, compare the "Current state" section below against the live
> code; on a mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED (the confirm-and-stop path can orphan processes if implemented naively)
- **Depends on**: plans/004-m4-ready-browser.md
- **Category**: dx
- **Planned at**: commit `e74666e`, 2026-08-05
- **Reconciled at**: commit `2243d40`, 2026-08-06 — this plan was authored before any
  code existed. The step-by-step body was written against SPEC.md and is still correct;
  the "Current state" and "Commands" sections below were refreshed against the real
  codebase after M1–M4, the CI work (plan 009) and the audit fixes (007/008/015).

## Why this matters

Until this plan lands, using Hangar means hand-editing `projects.json` — which is exactly the
terminal detour the whole product exists to remove. The one genuinely dangerous piece here is
Remove/Edit on a *running* project: a naive Remove deletes the registry entry while the
process tree keeps running, producing an orphan with no UI handle to kill it — the precise
failure the entire M3 effort was spent preventing. Editing the port mid-run breaks Stop's
port verification the same way.

## Current state

Plans 001–004 produced the scaffold and storage, the spawn helper and log pipeline, the kill
paths and state machine, and the full run sequence with ready-detection and browser hand-off.
Verified against the codebase at `2243d40`:

**Already implemented — do NOT rebuild these:**
- `src-tauri/src/main.rs` registers 10 commands: `get_projects`, `get_settings`,
  `set_settings`, `get_registry_error`, `run_project`, `stop_project`, `open_in_browser`,
  `get_log_buffer`, `clear_log_buffer`. **`get_settings`/`set_settings` already exist**
  (M1) — this plan builds their *dialog*, not the commands.
- `src/components/ProjectCard.tsx` overflow menu is a `MENU_ITEMS` array of
  `{ label, action }`; **"Open in browser" and "Show logs" are already wired**. Only
  "Open in editor", "Edit" and "Remove" have `action: null` and render disabled. Wire
  those three by giving them actions — do not restructure the array.
- `src/App.tsx` renders an Add button in both the empty state and the header; both are
  inert (documented at the top of the file as "open nothing yet (plan 005)").
- `src/components/AddEditDialog.tsx` and `SettingsDialog.tsx` exist as stubs.

**Things that will bite you if you don't know them:**
- **Wire-contract tests exist (plan 008).** `src-tauri/src/registry.rs` has
  `every_wire_key_the_backend_emits_appears_in_types_ts`, which asserts every JSON key the
  backend emits is declared as a property (`key:` / `key?:`) in `src/types.ts`. When you add
  §7 payloads (`read_package_json`'s return type, `NewProject`), you MUST add matching
  declarations to `src/types.ts` AND extend that test's sample list — otherwise the new
  shapes are simply uncovered. Its maintenance note says exactly this.
- `src-tauri/src/commands.rs` now has a test module (`to_view` derived-field tests). Keep
  them passing; `to_view` is what computes `path_exists`.
- Registry writes currently happen while the `projects` async mutex is held. Plan 010 (TODO)
  will reshape that to snapshot-then-save. **Match the existing shape** — do not pre-empt
  010, and do not make it worse by adding more work under the lock.
- `npm run dev` now means `tauri dev` (plan 009). `npm run dev:web` is bare Vite.

**Read before writing code**: SPEC.md §10 (add/edit/remove flow), §5 (data model, `url`
semantics, `pathExists`), §7 (frozen command API — the commands still MISSING are
`add_project`, `update_project`, `remove_project`, `read_package_json`, `open_in_editor`),
§6 (the Remove/Edit-while-running rule), §11 (dialog and settings UI — palette, fonts and
tokens are already defined in `src/index.css`; reuse them, no generic defaults).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Full verify (4 gates in one) | `npm run verify` | exit 0 |
| Rust tests | `npm run test:rust` | 76 pass, 3 ignored |
| Acceptance (Unix, serial) | `npm run test:acceptance` | 3 pass |
| TypeScript | `npm run typecheck` | exit 0 |
| Windows typecheck (incl. tests) | `PATH="/opt/homebrew/opt/llvm/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --all-targets` | exit 0 |

`cargo` lives in `~/.cargo/bin`; prefix `PATH="$HOME/.cargo/bin:$PATH"` if not found.

## Scope

**In scope**:
- `src-tauri/src/commands.rs`, `src-tauri/src/registry.rs` (validation + `read_package_json`)
- `src/components/AddEditDialog.tsx`, `src/components/SettingsDialog.tsx`,
  `src/components/ProjectCard.tsx` (overflow menu wiring), `src/api.ts`, `src/App.tsx`

**Out of scope**:
- `git pull`, lockfile hashing, `npm install`, the phase strip, the uptime slot, the log
  Copy button — all plan 006.
- Any change to the kill sequence or state machine — call them, do not modify them.
- Auto-discovery or scanning for projects — SPEC.md §3 forbids it outright.

## Git workflow

One commit at the end: `Add M5 project add/edit/remove, validation, and settings`.

## Steps

### Step 1: Implement `read_package_json`

Per SPEC.md §7 and §10. Return the `scripts` map, the detected package manager (from which
lockfile is present: `package-lock.json` → npm, `pnpm-lock.yaml` → pnpm, `yarn.lock` → yarn),
and an optional port suggestion from dependency sniffing: `next` → 3000, `vite` → 5173,
`react-scripts` → 3000, otherwise `None`.

A missing or unparseable `package.json` is **not** an error — it returns empty scripts so the
dialog falls back to manual command + port entry (§10 step 6).

Unit-test the sniffing and package-manager detection.

**Verify**: `cargo test` → tests pass for each sniffing case and each lockfile.

### Step 2: Implement registry validation

Per SPEC.md §10 step 5: **no two projects may register the same port** — the error names the
conflicting project. Per §10 step 5 also: two projects **may share a path** (e.g. `dev` and
`storybook` from one repo on different ports) — this is explicitly allowed and must not error.

Per §5: `url` is an optional override. Ready-check, busy-check, and duplicate-port validation
**always** use `port`, never the url. If a supplied url contains an explicit port different
from `port`, produce a **non-blocking warning**, not a rejection.

Validation lives in Rust so it cannot be bypassed by the frontend.

**Verify**: `cargo test` → tests cover: duplicate port rejected and names the owner; same
path with different ports accepted; url-port mismatch warns without blocking.

### Step 3: Build the Add/Edit dialog

Per SPEC.md §10 steps 1–4 and 6. Native folder picker via the dialog plugin (this one *is*
webview-initiated, so it needs the capability entry plan 001 created).

After picking a folder: call `read_package_json`, list scripts as selectable options,
pre-select `dev` if present else `start`, build the command as `npm run <script>` /
`pnpm run <script>` / `yarn <script>` per detected manager. **The command field stays
editable free text.** Port prefilled from the suggestion, otherwise empty and required —
a suggestion, never silent magic.

Include the §10 step 3 hint text under the command field about inline env vars
(`PORT=3001 npm run dev`), which is how v0 handles a framework that ignores the pinned port.

**Verify**: `npx tsc --noEmit` → exit 0; `npm run build` → exit 0.

### Step 4: Implement confirm-and-stop for Remove and Edit

Per SPEC.md §6 and §10 step 7. If status ∉ {`stopped`, `crashed`}, first show
`"<name> is running. Stop it first?"`. On confirm: run the **full plan 003 kill sequence and
wait for verified death**, then apply the remove or save.

The backend guard added in plan 003 must remain the real enforcement — the dialog is
courtesy, not security. Verify `remove_project` still rejects a running project if called
directly.

**Verify**: `cargo test` → the guard test from plan 003 still passes. Manual: start a
project, click Remove, confirm → process count returns to baseline and the card disappears.

### Step 5: Wire the overflow menu and settings

Per SPEC.md §10 step 7 and §11. **"Open in browser" and "Show logs" are already wired** —
give the remaining three (`Open in editor`, `Edit`, `Remove`) actions in the existing
`MENU_ITEMS` array in `src/components/ProjectCard.tsx`; do not restructure it.

`open_in_editor` runs `<editorCommand> <path>` **through plan 002's spawn helper** (never a
bare `Command`, which cannot execute `code` on Windows — it is a `.cmd` shim). On failure,
toast: `Couldn't run 'code' — is it on your PATH? Change the editor command in Settings.`
Never fail silently.

Settings dialog: one field, "Editor command", default `code`. Nothing else (§11).

Recompute `pathExists` at startup, on registry change, and when Run is clicked (§5) — a card
whose folder was deleted shows the warning state with Run disabled and Edit/Remove enabled.

**Verify**: `grep -rn "Command::new" src-tauri/src/ | grep -v process.rs` → still no matches;
`npx tsc --noEmit` → exit 0.

### Step 6: Run all gates, perform manual checks, and commit

Manual, recorded in your report:
- **§15 test 6**: attempt to register two projects on the same port → impossible, and the
  error names the conflicting project.
- Add a real project through the dialog → Run → Edit → Remove, without touching
  `projects.json` by hand at any point.

## Test plan

Rust unit tests (`cargo test`): port-suggestion sniffing per dependency; package-manager
detection per lockfile; duplicate-port rejection naming the owner; same-path acceptance;
url-port mismatch warning; missing `package.json` returning empty scripts rather than erroring.

Manual acceptance tests as in step 6.

## Done criteria

- [ ] `npm run verify` exits 0, and `npm run test:acceptance` still reports 3 passed
- [ ] The Windows typecheck command in the table above exits 0 (it now covers test code)
- [ ] Every new §7 payload is declared in `src/types.ts` AND added to the sample list in
      `every_wire_key_the_backend_emits_appears_in_types_ts` (plan 008's guard)
- [ ] `cargo test` passes with the validation and sniffing tests
- [ ] §15 test 6 verified manually and reported
- [ ] A project can be added, run, edited, and removed entirely through the UI
- [ ] `grep -rn "Command::new" src-tauri/src/ | grep -v process.rs` → no matches
- [ ] No git/install/phase-strip work was added
- [ ] `plans/README.md` status row for 005 updated

## STOP conditions

Stop and report back if:

- Removing a running project leaves any process alive. Report `pgrep -fl node` output.
- The dialog plugin's folder picker fails with a permission error — that means plan 001's
  capability file is wrong; report it rather than working around it by shelling out.
- You find yourself wanting to scan directories to find projects. SPEC.md §3 forbids
  auto-discovery outright — stop and flag it.
- Validation would need to reject two projects sharing a folder. It must not; §10 allows it.

## Maintenance notes

- Plan 006 adds the per-canonical-path mutex that makes two projects sharing a folder safe
  during concurrent pull/install. Until then, sharing a path is allowed but uncoordinated.
- A reviewer should scrutinize: that validation is enforced in Rust rather than only in the
  dialog, that confirm-and-stop waits for *verified* death, and that `open_in_editor` goes
  through the shared spawn helper.
