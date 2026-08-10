# Plan 034: A route from the toast to the log, plus three validation bugs

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "function Toast" src/App.tsx && grep -n "url_port_mismatch_warning" src-tauri/src/registry.rs`
> Both must match. On a mismatch, STOP.

## Status

- **Priority**: P2 — fix 1 cost the maintainer a real diagnosis cycle on
  2026-08-10; the other three are latent
- **Effort**: S
- **Risk**: LOW — no schema change, no §7 change, no state-machine change
- **Depends on**: nothing
- **Category**: bug + dx
- **Planned at**: 2026-08-10, after the ground-truth gap analysis

## Why this matters

All four came out of a gap analysis grounded in the maintainer's real registry,
then adversarially verified. Fix 1 is the one that already cost time.

### Fix 1 — the toast tells you to read the log and gives you no way to reach it

Three of the app's best messages end by pointing at the log:

- `src-tauri/src/run.rs:1258-1259` — "Install failed (exit n) — see the log, then Run again."
- `src-tauri/src/run.rs:347-352` — the §9 step 7 ready-timeout: "Check the log — did it start on another port?"
- `src-tauri/src/run.rs:374-388` — `starting_exit_message`

`Toast` (`src/App.tsx:82-101`) renders a `<span>` and a Dismiss `✕`. Nothing
else. Reaching the log means finding the card, opening `⋯`, and clicking "Show
logs" — item 4 of 7 in `MENU_ITEMS` (`src/components/ProjectCard.tsx:94-104`).

**This happened for real on 2026-08-10.** A ready-timeout fired on
`auto-job-applier web`; the toast asked "did it start on another port?"; the
answer (Vite's `Local:` line) was sitting in the log the whole time.

The toast also names no project. Both of the maintainer's Node projects run the
identical command string `npm run dev`, so an unattributed failure is ambiguous
on its face.

**Not a bug, verified:** the install path toasts twice — `crash_run`
(`run.rs:1047-1050`) emits the `crashed` event *and* returns `Err` with the
**same** string, which `startProject`'s catch (`src/store.ts:441-449`) toasts
again. Identical text into one slot, so nothing is lost. But the catch's toast
carries no project id, so if it lands second it would erase the button this plan
adds. Step 2 handles that.

### Fix 2 — the Add dialog silently keeps the previous project's port

`src/components/AddEditDialog.tsx:120`:

```tsx
if (info.portSuggestion !== undefined) setPort(String(info.portSuggestion));
```

The **false** branch leaves whatever was in the box. Browse a Vite app → 5173.
Re-browse a project with no detectable port → the box still reads 5173,
presented as a suggestion for the new folder. SPEC.md §10 step 4 calls the
prefill "a suggestion, never silent magic"; this is silent magic.

The dialog's re-init effect (`AddEditDialog.tsx:56-59` region) also resets nine
setters but leaves `scripts`, `selectedScript` and `packageManager` holding the
previous project's values.

### Fix 3 — a §5 obligation that never ships, and a doc comment that says it does

SPEC.md §5: *"If a provided `url` contains an explicit port different from
`port`, show a non-blocking validation warning: 'URL port differs from the
ready-check port.'"*

`url_port_mismatch_warning` (`src-tauri/src/registry.rs:393-404`) is correct and
tested — and marked `#[allow(dead_code)]` at line 392. Its doc comment
(`registry.rs:389-391`) asserts:

> `AddEditDialog.tsx` mirrors this exact wording as the user types, live

`grep -rn "URL port differs" src/` returns **nothing**. The claim is false. The
warning has never shipped, and the comment is why nobody noticed.

### Fix 4 — `readyTimeoutSec: 0` kills the tree on the first poll

`canSave` (`AddEditDialog.tsx:135-137`) checks name, path, command and port —
**not** `readyTimeoutSec`. The input's `min={1}` is advisory only; React does not
enforce it and `commands.rs:106` passes the value straight through.

`AttemptBudget::new(0)` → `remaining: 0` → `await_ready` probes once,
`is_exhausted()` is true, `TimedOut`, and the tree is killed on the first poll
(`run.rs:249-256`, `run.rs:293-319`). Latent — nobody has hit it.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |
| Bundle | `npm run build` | exit 0 |
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --all-targets` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass (baseline 116 — **run it first and report what you observe**) |

**Do not run** `npm run verify`, `npm run build:app`, or
`npm run test:acceptance` — a 600 s no-output watchdog has killed executor runs
here. Keep every Write/Edit under ~60 lines and commit after each.

## Scope

**In scope**:
- `src/store.ts` — the toast's optional project id; the `startProject` catch
- `src/App.tsx` — the `Toast` component
- `src/components/AddEditDialog.tsx` — fixes 2, 3, 4
- `src-tauri/src/registry.rs` — fix 3's doc comment and `#[allow(dead_code)]`
- `src-tauri/src/commands.rs` — fix 4's backend floor, if you add one

**Out of scope** (do NOT touch):
- `src-tauri/src/run.rs`. The messages are good; the problem is the route to the
  log, not the wording. Do not reword them.
- The §6 state machine, §8 kill paths, §9 run sequence, `AttemptBudget` itself.
- Any §7 command signature. §7 is FROZEN. `openLogs` and `get_log_buffer`
  already exist (`src/store.ts:509` region) — reuse them.
- `MENU_ITEMS` and the card. The overflow route stays exactly as it is.
- The folder feature, the drag subsystem, `gridItems`.
- Stack detection (`detect_stack`, the allow-lists). A separate plan owns it and
  may be running concurrently — touching `registry.rs`'s detection functions
  causes a conflict. Fix 3 touches only `url_port_mismatch_warning`'s attribute
  and doc comment.
- Any new dependency.

## Git workflow

- One commit per step: `Toast/validation: <what>`.

## Steps

### Step 1: Give the toast an optional project

In `src/store.ts`:

- Add `toastProjectId: string | null` to `HangarState`, initialised to `null`.
- Extend `setToast(message, tone = "error", projectId?: string)` — a **third
  optional parameter**, so all 15 existing call sites stay textually unchanged.
  Clear it (`null`) whenever `projectId` is absent, so a later generic toast
  cannot inherit an earlier project's button.

**Verify**: `npm run typecheck` → exit 0.

### Step 2: Pass the project id where a project is known

Two call sites, both in `src/store.ts`:

- The `crashed` event handler (`store.ts:400-402`) — pass `payload.projectId`.
- `startProject`'s catch (`store.ts:441-449`) — pass its `projectId`. **This one
  matters even though its text is identical to the crashed toast**: without it,
  whichever lands second decides whether the button exists.

Do not touch the other 13 call sites.

**Verify**: `npm run typecheck` → exit 0.

### Step 3: Render the button, and name the project

In `src/App.tsx`'s `Toast`:

- Prefix the message with the project's name when `toastProjectId` resolves to a
  project in the store — `"<name> — <message>"`. If it does not resolve (removed
  meanwhile), render the message alone. Never render a dangling name.
- Add a **Show logs** button, before Dismiss, only when the id resolves. It calls
  the existing `openLogs(projectId)` and then `setToast(null)` so the toast does
  not sit on top of the panel it just opened.
- Style it as a quiet text button using existing tokens — no raw hex, no new
  palette entry. It must not compete with Dismiss for the eye.

§11 fixes the **card's** element list, not the toast's, and §11 requires "errors
always say what happened and what to do next" — this is that rule being
delivered, not a new element. No amendment.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 4: Fix the stale prefills (fix 2)

In `AddEditDialog.tsx`:

- `handleBrowse`: when `info.portSuggestion` is `undefined`, **clear** the port
  field rather than leaving the previous project's value. An empty box the user
  must fill is honest; a stale number presented as a suggestion is not.
- The dialog's open/reset effect: also reset `scripts`, `selectedScript` and
  `packageManager` alongside the nine setters already there.
- The `catch` branch at `AddEditDialog.tsx:122-127` already clears `scripts` and
  `selectedScript`; make it consistent with whatever you do above.

**Verify**: `npm run typecheck` → exit 0.

### Step 5: Ship the URL-port warning (fix 3)

- In `AddEditDialog.tsx`, render a **non-blocking** warning beneath the URL field
  when the URL carries an explicit port different from the port field. Exact
  wording, from SPEC.md §5: `URL port differs from the ready-check port.`
  It must **never** block Save — §5 says non-blocking, and `canSave` must not
  learn about it.
- Port extraction is a few lines of TS; do not add a dependency and do not add a
  §7 command to ask Rust. Mirror `extract_url_port`'s deliberate narrowness
  (`registry.rs:406` region) — read it first so the two agree.
- In `src-tauri/src/registry.rs`: the function is now genuinely mirrored, so
  **rewrite the doc comment to say what is true** — that the TS mirror lives in
  `AddEditDialog.tsx` and this stays the tested canonical definition. Keep
  `#[allow(dead_code)]` (it is still uncalled from Rust) and say why in one line.

**Verify**: `npm run typecheck` → exit 0; `cargo check --all-targets` → exit 0;
`grep -rn "URL port differs" src/` → **one** match.

### Step 6: Floor the ready timeout (fix 4)

- Frontend: include `readyTimeoutSec >= 1` in `canSave`, and extend the disabled
  Save tooltip to mention it. An integer ≥ 1.
- Backend: reject or clamp a value below 1 in `add_project`/`update_project`.
  Prefer **reject with a clear message** over silent clamping — §10 step 4's
  "never silent magic" applies to values as much as to prefills. Add a unit test
  for whichever you choose.

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → all pass, report
the new total; `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 7: Gates and self-check

Report each:

- `grep -n "toastProjectId" src/store.ts src/App.tsx` → present in both.
- `grep -c "setToast(" src/store.ts` → unchanged from the baseline you recorded.
- `grep -rn "URL port differs" src/ src-tauri/src/` → one TS match, one Rust match.
- `grep -rnE "#[0-9A-Fa-f]{6}" src/App.tsx src/components/AddEditDialog.tsx` → none.
- `git status --short` → only in-scope files.

**Verify**: all four gate commands green.

## Test plan

Rust: a unit test for fix 4's validation, and the existing
`url_port_mismatch_warning` tests keep passing untouched.

No JS test runner for component behaviour (SPEC.md §4). Manual checks for the
maintainer:

- Force a ready-timeout (set a card's port to something nothing binds, Run). The
  toast reads `<project name> — Server didn't answer on port …` and has a **Show
  logs** button that opens that project's log.
- Add a project, Browse a Vite app (port fills 5173), then Browse a project with
  no detectable port → the port box is **empty**, not 5173.
- In Edit, set URL to `http://localhost:4000` while the port field says 5175 →
  the warning appears and Save still works.
- Set Ready timeout to 0 → Save is disabled with a tooltip saying why.

## Done criteria

- [ ] All four gates green; report `cargo test` before/after counts
- [ ] The toast names its project and offers Show logs when a project is known
- [ ] The port prefill never carries a previous project's value
- [ ] `grep -rn "URL port differs" src/` finds the TS mirror
- [ ] `readyTimeoutSec` below 1 cannot be saved
- [ ] `run.rs` unmodified; no §7 change; no new dependency
- [ ] `plans/README.md` status row for 034 updated

## STOP conditions

Stop and report back if:

- The toast button appears to need a §7 command. It does not — `openLogs` exists.
- Fix 3 appears to need a Rust round trip or a URL-parsing dependency. It needs
  neither; §5's rule is a string comparison on an explicit `:port`.
- Fixing fix 2 requires reshaping the dialog's state model. Report instead.
- You find yourself editing `run.rs`'s message strings, `detect_stack`, or the
  library allow-lists. All are out of scope and the last two belong to a
  concurrent plan.

## Maintenance notes

- **The lesson from fix 3 is the one worth keeping**: a doc comment asserted that
  a frontend mirror existed, and that assertion is precisely why nobody checked.
  A comment that claims another file does something is a claim, not documentation
  — and this repo has no gate that can check it. Prefer a test, or say "should"
  rather than "does".
- Fix 1's shape generalises: any toast that mentions the log should carry the
  project it belongs to. If a fourth "see the log" message appears, it needs a
  project id at its call site or the button silently will not render.
