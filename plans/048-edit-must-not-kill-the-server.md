# Plan 048: Opening Edit must not kill your dev server

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise. Update this plan's row in
> `plans/README.md` when done, unless a reviewer told you they maintain it.
>
> **Drift check**: `grep -n "stopIfRunningWithConfirm" src/components/ProjectCard.tsx src/store.ts && grep -n "fn is_run_inert_change" src-tauri/src/commands.rs`
> All must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. Your reviewer runs `npm run build`.

## Status

- **Priority**: P2
- **Effort**: S-M
- **Risk**: MED — touches §6's mutation guard, one of CLAUDE.md's
  highest-priority correctness requirements
- **Depends on**: SPEC.md §6 amendment (this plan writes it — see step 1)
- **Category**: bug
- **Planned at**: commit `da973be`, 2026-08-10

## Why this matters

`handleEdit` (`src/components/ProjectCard.tsx`) calls
`stopIfRunningWithConfirm` **unconditionally, before the dialog opens**. So on
a running project, choosing `⋯ → Edit` prompts *"… is running. Stop it first?"*
and, on confirm, **actually stops the server**.

The cost is concrete. The maintainer gained an `openBrowserOnReady` checkbox
today, specifically for his API-only server that opens a junk `Cannot GET /`
tab on every Run. The moment he notices that tab is while the server is
running — and the only control is inside Edit. So flipping one checkbox costs
him the process plus a full Pull → Install → Start to get it back.

**§6's own wording is "before removing/saving"** — not before *looking*.

## The two changes

### A — the confirm moves from dialog-open to Save

The frontend is currently **stricter than the backend**. `guard_update`
already returns `Ok` immediately for a run-inert change, so a notes-only or
folder-only save on a running project is permitted by Rust and blocked by the
UI before the dialog even appears.

Open the dialog freely. Run the existing confirm-and-stop **only when Save is
pressed and the pending change is not run-inert.**

### B — `openBrowserOnReady` joins the run-inert set

It is read at exactly **one** place: `run.rs`'s ready hand-off, inside a
function that receives `Project` **by value** — a snapshot taken at step 0,
before the run began. A mid-run write is therefore structurally unobservable by
the current run; it can only ever affect the next one.

That is the proof the §6 amendment needs, and it must be written into the
amendment as *"read only from the pre-run snapshot"* — **not** as "read late",
which would invite the next field in on weaker grounds.

**Pin it with a test**, because the risk is a later refactor that re-reads the
record mid-run.

### The asymmetry to fix while you are here

`AddEditDialog` always sends `openBrowserOnReady`, seeded `?? true`, and every
stored record currently holds `None`. So `Some(true) != None` makes **every**
save from that dialog a guarded change today. Send `undefined` when the value
matches the effective default, or normalise `None`/`Some(true)` as equal in the
comparison — pick one, and say which and why.

## Scope

**In scope**: `SPEC.md` §6, `src-tauri/src/commands.rs` (the run-inert set and
its tests), `src/components/ProjectCard.tsx` (`handleEdit`),
`src/components/AddEditDialog.tsx` (the save path), `src/store.ts` if the
confirm helper needs a second entry point.

**Out of scope**: `run.rs` — its read site is correct and must not move.
`guard_mutation`, the §6 state machine, §8 kill paths, §9. `remove_project`'s
confirm — Remove is destructive and keeps its guard exactly as it is. No new
§7 command. No new dependency.

## Steps

### Step 1: Amend §6

Add `openBrowserOnReady` to the run-inert set, with the justification above.
The sentence must say **"read only from the pre-run snapshot"**, and must keep
the existing rule that the comparison is structural and a new field is guarded
by default.

**Verify**: `grep -n "openBrowserOnReady" SPEC.md` shows it in §6's run-inert
sentence.

### Step 2: Backend

Add the field to `is_run_inert_change`'s normalisation and to
`merge_run_inert_fields`. Both already handle three fields; this is a fourth.

Resolve the `Some(true)` vs `None` asymmetry (see above).

Tests:
1. An `openBrowserOnReady`-only change is run-inert → `Ok` while `Running`.
2. It paired with a `port` change is **still guarded**.
3. A merge writes it without disturbing `lastRunAt`/`stack`.
4. `Some(true)` vs `None` does not by itself make a change guarded.

Then delete the new normalisation line, confirm test 1 **fails**, restore.
**Report both outcomes.**

**Verify**: `cargo check --all-targets` → 0; `cargo test` → report before/after.

### Step 3: Frontend

`handleEdit` opens the dialog directly — no confirm, no stop.

`AddEditDialog`'s save path runs `stopIfRunningWithConfirm` **only** when the
change is not run-inert. The frontend cannot call Rust's predicate, so compute
the equivalent locally: compare the outgoing record against `editing` with the
four run-inert fields normalised out. **Put the field list in one named
constant** used by that comparison, and comment that it mirrors §6's sentence —
two lists that can drift is exactly how this bug class returns.

If the comparison says "guarded", behaviour is unchanged: confirm, stop, wait,
then save. If it says run-inert, save straight through.

**Verify**: `npm run typecheck` → 0.

### Step 4: Self-check

- `grep -n "stopIfRunningWithConfirm" src/components/ProjectCard.tsx` → **no
  match in `handleEdit`** (Remove keeps its own).
- `grep -c "open_browser_on_ready" src-tauri/src/run.rs` → unchanged from before
  your work.
- `git diff --stat` → `run.rs` untouched.

**Verify**: all three gates green.

## Test plan

Manual, for the reviewer/maintainer:

- With `auto-job-applier server` **running**, open `⋯ → Edit`. **No prompt, the
  server keeps running.** Untick *Open browser when ready*, Save. It saves, the
  server is still up, and the next Run opens no tab.
- With it running, open Edit and change the **port**. Save → the old
  confirm-and-stop appears, exactly as before.
- Notes autosave during a run still works.
- Remove on a running project still confirms and stops.

## Done criteria

- [ ] Three gates green; `cargo test` before/after reported
- [ ] The mutation test in step 2 was run and both outcomes reported
- [ ] Editing a running project no longer stops it; a guarded field still does
- [ ] `run.rs` untouched
- [ ] `plans/README.md` row updated

## STOP conditions

- The frontend seems to need a §7 command to ask Rust whether a change is
  run-inert. It does not — mirror the comparison locally, from one named list.
- `openBrowserOnReady` turns out to be read anywhere other than the pre-run
  snapshot. Then the §6 justification is false and the amendment must not ship;
  report instead.
- Remove starts skipping its confirm. Remove is destructive and out of scope.

## Maintenance notes

- The run-inert set now exists in **two** places: §6's sentence and a frontend
  constant. That duplication is deliberate (the frontend cannot call Rust's
  predicate) and it is the thing to check first if this bug ever returns.
- The general rule worth keeping: **a guard that runs before the user has
  expressed intent is guarding the wrong moment.** §6 says "before
  removing/saving"; anything earlier is the UI being stricter than the contract.
