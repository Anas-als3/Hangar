# Plan 045: Stop opening a browser tab on every single Run

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "openBrowserOnReady" SPEC.md` — §5 and §9
> step 6 must both mention it. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. **Your reviewer runs `npm run build` and the bundle.**
>
> **Supersedes** the `040` row in `plans/README.md`, which was a placeholder
> opened before the amendments existed. Do not write a plan 040.

## Status

- **Priority**: P2 — the one moment Hangar is actively *worse* than a terminal
- **Effort**: S
- **Risk**: LOW-MED — touches `run.rs`'s ready path, which is §9's core
- **Depends on**: SPEC.md §5/§9 step 6/§10 amendments (ratified 2026-08-10),
  plan 043 (merged — both touch `AddEditDialog.tsx`)
- **Category**: bug
- **Planned at**: 2026-08-10

## Why this matters

`src-tauri/src/run.rs` opens the browser unconditionally when a project reaches
ready. Two costs, both observed on the maintainer's real registry:

1. **`auto-job-applier server` is an API-only Express server.** It mounts
   `/api` and `/files` and nothing at `/`. So **every Run since it was added**
   has opened a browser tab reading `Cannot GET /`. There is no page to show and
   there never will be.
2. **Stop → Run on a live-reloading dev server opens a duplicate tab.** The
   already-open one reconnected on its own; the new one is junk the user closes.

This is the single place the app is worse than typing `npm run dev`, because a
terminal does not open anything.

**Read SPEC.md §9 step 6 and §5's `openBrowserOnReady` line before starting.**
Both were amended for this plan.

## The rule that must not be bent

> The status transition is unconditional; **only the browser hand-off is
> skipped**, and the run log still records readiness.

`running` must still be reached, the phase strip must still light Ready, and the
log must still say the server answered. A project that opts out is not a project
that runs differently — it is a project whose tab you don't want.

And the default stays `true`. §9 step 6 says so in writing: opening the browser
is §2's payoff, and a silent opt-out would be worse than a junk tab.

## Current state

`src-tauri/src/run.rs` — the ready hand-off (search for the `opened {url}`
system line and the opener call around it):

```rust
            process::append_system(app, &project.id, format!("opened {url}")).await;
```

`src-tauri/src/registry.rs` — `Project`, ending with the folder fields added by
plan 028. Optional fields there use
`#[serde(default, skip_serializing_if = "Option::is_none")]`.

`src/components/AddEditDialog.tsx` — has checkbox precedent: `updateOnRun`.
Match it.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --all-targets` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass (baseline 143 — **run it first and report what you observe**) |
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run build`, `npm run build:app`, `npm run install:app`,
`npm run verify`, or `npm run test:acceptance`. Keep every Write/Edit under ~60
lines and commit after each.

## Scope

**In scope**:
- `src-tauri/src/registry.rs` — the field, the fixtures, the drift-guard sample
- `src-tauri/src/run.rs` — the conditional at the hand-off **only**
- `src/types.ts` — the mirrored field
- `src/components/AddEditDialog.tsx` — the checkbox

**Out of scope** (do NOT touch):
- **The `running` transition, the phase strip, the ready poll, the grace wait,
  the timeout path, the kill paths.** Only the browser call becomes conditional.
- `open_in_browser` — the §7 command behind the card's port button and the
  overflow menu. **That is an explicit user action and must always work**, even
  on a project with `openBrowserOnReady: false`. If you make it conditional you
  have removed the user's only way to open the page at all.
- The §6 state machine, §8 kill paths, `port_conflict`, the ports panel,
  `find_free_port`, `portToken.ts`.
- Any new §7 command. `update_project` already carries the whole record.
- The default. It is `true` and §9 step 6 forbids changing it.
- Any new dependency.

## Git workflow

- One commit per step: `Browser opt-out: <what>`.

## Steps

### Step 1: The field

Add to `Project` in `registry.rs`, after the folder fields:

```rust
/// SPEC.md §5 / §9 step 6 (added 2026-08-10): when `false`, reaching ready does not hand off to
/// the browser. The status transition is unaffected — only the tab is skipped. `None` means the
/// default, `true`: absent from an existing `projects.json`, and absent from the file for every
/// project that never turns it off.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub open_browser_on_ready: Option<bool>,
```

Mirror on `NewProject` with `#[serde(default)]`, carry it through `add_project`,
and mirror in `src/types.ts`.

**Repair every exhaustive literal.** `grep -rn "ready_timeout_sec:" src-tauri/src/`
finds them: `seed_projects`, `sample()`, `add_project`, `commands.rs`'s
`sample_project`, `run.rs`'s `project_fixture`.

Two need more than `None`:

- **`registry.rs`'s `sample()` must set it to `Some(...)`** or
  `skip_serializing_if` omits the key and the drift guard passes without
  checking anything. Read the comment already above `notes` there — it explains
  this trap.
- **`project_view_flattens_project_and_adds_derived_fields` asserts an exact
  JSON string** in field-declaration order. Extend it in the right position.

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass with no
expectation edits beyond the exact-JSON one. Then mutation-test the guard:
delete `openBrowserOnReady` from `types.ts`, confirm the guard **fails**,
restore, confirm it passes. **Report both outcomes.**

### Step 2: The conditional hand-off

In `run.rs`, guard **only** the browser call. `unwrap_or(true)` is the default.

Requirements:

- The `running` transition happens either way, unchanged.
- **The log still records readiness either way.** When the hand-off is skipped,
  append a `system` line saying so — something a user reading the log can
  understand, e.g. `ready on http://localhost:4000 — browser opt-out is on for this project`.
  Silence here would look like a failure.
- Do not restructure the surrounding function. This is a conditional around one
  call and one log line.

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass;
`grep -n "open_browser_on_ready" src-tauri/src/run.rs` → exactly one site.

### Step 3: The checkbox

In `AddEditDialog.tsx`, add **Open browser when ready**, default checked,
following `updateOnRun`'s existing markup exactly — same row shape, same label
style, same state wiring.

Place it beside `updateOnRun`. One line of helper text: `Turn this off for a project with no page to serve, like an API-only server.`

**Verify**: `npm run typecheck` → 0.

### Step 4: Self-check

Report each:

- `grep -c "open_browser_on_ready" src-tauri/src/run.rs` → 1.
- `grep -n "open_in_browser" src-tauri/src/commands.rs` → unchanged; the
  explicit action is **not** gated.
- `grep -n "openBrowserOnReady" src/types.ts src/components/AddEditDialog.tsx` → present in both.
- `git status --short` → only in-scope files.

**Verify**: all three gates green.

## Test plan

Rust: extend the exact-JSON view test (step 1). No new behavioural test — the
conditional needs an `AppHandle`, which nothing in this codebase constructs
outside a running app (the constraint plan 020 already recorded).

Manual checks for the reviewer/maintainer:

- Edit `auto-job-applier server`, untick **Open browser when ready**, Run it.
  **No tab opens**, the card still reaches `Running`, the phase strip still
  lights Ready, and the log says it was ready and that the opt-out is on.
- On that same project, the card's `:4000` port button and **Open in browser**
  still open the page. That is the explicit action and it must always work.
- Run IELTS Coach (checkbox left on) → the tab opens exactly as before.
- Check `projects.json`: projects that never touched the setting have **no**
  `openBrowserOnReady` key at all.

## Done criteria

- [ ] All three gates green; report `cargo test` before/after
- [ ] The drift-guard mutation test was run and both outcomes reported
- [ ] `running`, the phase strip and the ready log line are unaffected
- [ ] `open_in_browser` is not gated
- [ ] The default is `true`; an untouched `projects.json` gains no key
- [ ] `plans/README.md` status row for 045 updated

## STOP conditions

Stop and report back if:

- Making the hand-off conditional appears to need a change to the `running`
  transition, the ready poll, or the timeout path. It does not — it is one
  `if` around one call.
- You are tempted to gate `open_in_browser` as well. That would leave a project
  with **no** way to open its page.
- The field seems to need to be non-optional. It must be `Option<bool>` so an
  existing `projects.json` is byte-identical until the user changes something.

## Maintenance notes

- The distinction worth preserving: **ready is a fact, the tab is a
  preference.** Anything that conflates them will re-introduce this bug from the
  other direction — e.g. by skipping the `running` transition to avoid the tab.
- If a "Run without opening" one-off is ever wanted, that is a different
  feature (a modifier on the Run button), not a change to this persisted
  preference.
- This is the fourth optional field added to `Project` with
  `skip_serializing_if` (`notes`, `stack`, the folder pair, now this). The
  pattern is load-bearing for §16's still-parked versioned wrapper: as long as
  additions stay optional and omitted-when-unset, the file stays a bare array
  and no migration is needed.
