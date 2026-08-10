# Plan 051: Resume last session, and a workspace strip

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "Resume last session" SPEC.md && grep -n "AddTile" src/components/ProjectGrid.tsx`
> Both must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `npm run typecheck`. **Your reviewer runs
> `npm run build`, `cargo check` and the bundle.**

## Status

- **Priority**: P2 — the maintainer's direct ask, twice: fill the space, and
  give him a reason to open the app first thing
- **Effort**: M
- **Risk**: LOW — both elements are derived from data already in the store; no
  Rust, no persisted state, no new command
- **Depends on**: SPEC.md §11 amendments (ratified 2026-08-10, `bcc8233`)
- **Category**: feature
- **Planned at**: commit `bcc8233`, 2026-08-10

## Why this matters

Two asks, in his words: *"we need to fill it more"*, and *"how can I make this
program the first program any developer would run after he starts his
computer."*

At three projects on a 1512-logical fullscreen, ~70 % of the content box is
empty. And nothing in the app is worth *reading* — only things worth clicking,
which is why there is no reason to open it first.

**Read SPEC.md §11's two new bullets — "Resume last session" and "Workspace
strip" — before starting.** They were written for this plan and they carry the
constraints.

### The finding that makes the first one cheap

`lastRunAt` is already stored per project. The set of projects that were
running together is therefore **derivable**: those whose `lastRunAt` falls
within a short window of the most recent one. Verified against the live
registry — it selects IELTS Coach and auto-job-applier web (seven seconds
apart) and correctly excludes auto-job-applier server (six hours earlier).

**No new `Project` field.** That is not tidiness: `is_run_inert_change`
compares cloned records structurally, so a new field would make every session
write a guarded mutation and break note-saving during a run.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |
| Pure-logic tests | `node --test src/session.test.mjs` | all pass |

**Do not run** `npm run build`, `npm run build:app`, `npm run install:app`,
`cargo` anything, `npm run verify`, or `npm run test:acceptance`. Keep every
Write/Edit under ~60 lines and commit after each.

## Scope

**In scope**:
- `src/session.ts` — new, **zero imports**, the clustering + inventory logic
- `src/session.test.mjs` — new, `node --test`
- `src/App.tsx` — the resume line above the grid
- `src/components/ProjectGrid.tsx` — the workspace strip after the Add tile
- `src/store.ts` — only if a trivial selector is needed

**Out of scope** (do NOT touch):
- **Anything under `src-tauri/`.** No Rust, no new command, no schema change.
  Both elements are derived from `ProjectView[]` the store already holds.
- The grid track, the card, the phase strip, the status pill, folders, the drag
  subsystem, the Ports panel, the toast.
- Any new persisted state, any `Project` field, any `localStorage`.
- Batching Run. "Start both" is **N sequential `run_project` calls** through
  the existing `startProject` action — no new §6 behaviour, no new command.
- Any colour outside the existing tokens. No gradient, no blur, no glass.
- Any motion. Both elements render statically.
- Any new dependency.

## Git workflow

- One commit per step: `Session: <what>` / `Workspace: <what>`.

## Steps

### Step 1: The pure module

New `src/session.ts` with **zero imports** — same arrangement as
`src/dragGeometry.ts` and `src/portToken.ts`, so `node --test` can reach it.
Read one of those first and copy the shape.

Export two functions taking plain data (id, name, status, lastRunAt, path,
stack) — **not `ProjectView`**, so the module stays import-free:

1. **`lastSessionCluster(projects, windowMs)`** → the ids of projects whose
   `lastRunAt` is within `windowMs` of the most recent `lastRunAt`, in
   `projects` array order. Default window: 30 minutes.
   - Projects with no `lastRunAt` are excluded.
   - An unparseable `lastRunAt` is excluded, never a crash.
   - Empty input, or all-missing timestamps, yields an empty array.
2. **`stackInventory(projects)`** → the deduped, counted inventory across the
   library: each framework and library name with the number of **distinct
   `path`s** it appears under.
   - **Dedupe by `path`, not by project.** Two cards sharing a repo root must
     count once — the maintainer has exactly that case.
   - Order: highest count first, then the order the names already appear in
     (the backend already orders services before frameworks — do not re-sort
     alphabetically and destroy that).
   - Return the full list; capping is the caller's job.

**Verify**: `npm run typecheck` → exit 0.

### Step 2: Tests for both

New `src/session.test.mjs`. At minimum:

1. Two projects seconds apart plus one six hours earlier → the two.
2. All timestamps missing → empty.
3. One project only → that one.
4. A malformed timestamp → excluded, no throw.
5. Two projects sharing a `path` with overlapping stacks → each name counted
   **once**.
6. A name in two different paths → counted twice.
7. Empty input → empty output, both functions.

**Verify**: `node --test src/session.test.mjs` → all pass; report the count.

### Step 3: The resume line

In `src/App.tsx`, above the grid.

Renders **only** when all three hold:

- no project is currently non-`stopped` (nothing running, starting, stopping…),
- the search box is empty,
- the cluster has at least one project.

Content: `Last session, 14 h ago · IELTS Coach · auto-job-applier web` and one
button, `Start both` (or `Start all N` above two names). List at most three
names, then `+N`.

Behaviour: the button calls the existing `startProject` for each id **in
sequence** — `for … of` with `await`, never `Promise.all`. Two reasons: §9's
per-path mutex means parallel starts on one repo serialise anyway, and
sequential keeps the toast slot from being overwritten by a second failure.

The relative time reuses `relativeTime` from `src/status.ts` — do not write a
second formatter.

It disappears the moment anything starts, because the first condition stops
holding. That is the design, not a side effect.

**Verify**: `npm run typecheck` → exit 0.

### Step 4: The workspace strip

In `src/components/ProjectGrid.tsx`, **after** `<AddTile />`, inside the grid
container, as a `col-span-full` block.

That position is load-bearing: it occupies **no grid cell at any project
count**. The Add tile already renders outside the map, and `col-span-full` in
this exact grid is proven by the open-folder band in the same file — read both
before writing.

Three lines, quiet:

1. A hairline rule and the word `Workspace` (Space Grotesk, muted).
2. `3 projects · 2 repos` — repos counted by **distinct `path`**.
3. The inventory: `TypeScript ×2 · React ×2 · Vitest ×2 · Express · OpenAI · …`
   Cap at 12 names then `+N`. Counts of 1 show no `×`.

**Do not** show any status, port, uptime, or anything that changes while a
project runs. §11 says the cards own all of that, and a strip that mutates
during a run competes with the pill.

Hide the whole strip when the registry is empty (the empty state owns that
screen) and when a search is active (the grid is showing a filtered subset, so
a library-wide inventory would be a lie).

**Verify**: `npm run typecheck` → exit 0.

### Step 5: Self-check

Report each:

- `grep -n "^import" src/session.ts` → **no output**.
- `grep -rn "gradient\|backdrop-blur\|glass" src/` → no matches.
- `grep -n "Promise.all" src/App.tsx` → no match in the resume handler.
- `grep -rn "localStorage\|sessionStorage" src/` → no matches.
- `git status --short` → only in-scope files; nothing under `src-tauri/`.

**Verify**: `npm run typecheck` → 0; `node --test src/session.test.mjs` → pass.

## Test plan

Step 2's cases are the machine-checkable part. Manual checks for the
reviewer/maintainer:

- With all three projects stopped, the resume line names **IELTS Coach and
  auto-job-applier web** — not the server, which last ran hours earlier.
- Press **Start both** → both start, then the line disappears.
- Type in the search box with everything stopped → the line disappears.
- The workspace strip reads `3 projects · 2 repos` — two, because two cards
  share `/Users/anas/Projects/auto-job-applier`.
- Its inventory shows `TypeScript ×2` and `React ×2`, not `×3`.
- Run one project → the strip does **not** change.
- Remove all projects → neither element renders; the empty state is unchanged.

## Done criteria

- [ ] `npm run typecheck` exits 0; `node --test` passes with ≥7 cases
- [ ] `src/session.ts` has zero imports
- [ ] Nothing under `src-tauri/` modified; no new persisted state
- [ ] The resume line hides when anything runs or a search is active
- [ ] The strip occupies no grid cell and shows nothing status-dependent
- [ ] `plans/README.md` status row for 051 updated

## STOP conditions

Stop and report back if:

- Either element seems to need a `Project` field, a §7 command, or Rust. Both
  are derived from what the store already holds.
- "Start both" seems to want a batching command. It is N sequential calls to
  the existing action.
- The strip cannot be placed without displacing a card. Report — the whole
  point of its position is that it costs no cell.
- You are tempted to sort the inventory alphabetically. The backend already
  orders external services ahead of frameworks; alphabetical would destroy that
  and put `@types/...`-shaped names first.

## Maintenance notes

- The cluster window (30 minutes) is a guess that has never met real use. If it
  ever groups two unrelated sessions, that number is the one knob — do not add
  persisted session state to fix it before trying a smaller window.
- Both elements are derived. If either ever needs to be stored, that is a §5
  change and it re-opens the run-inert question the amendment avoided.
- The strip's position after `<AddTile />` is what makes it free. Anything that
  moves it into the map costs a grid cell at every project count.
