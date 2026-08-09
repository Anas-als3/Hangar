# Plan 022: Stop the app losing information the user needs — five visibility fixes

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat cc6172c..HEAD -- src-tauri/src/run.rs src/store.ts src/App.tsx src/components/ProjectCard.tsx`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW — five small, independent fixes. None changes a state machine,
  a kill path, or the persisted schema.
- **Depends on**: none. Runs safely in parallel with plan 021 (no shared files).
- **Category**: bug
- **Planned at**: commit `cc6172c`, 2026-08-09

## Why this matters

A 2026-08-09 gap analysis found that Hangar repeatedly *computes* something the
user needs and then throws it away. Five instances, each small, with a common
theme: information exists for a moment and then becomes unrecoverable.

Two of the five are regressions this project introduced itself and missed in
review. All five are fixes against what SPEC.md already promises — **none needs
a spec amendment**, which is why they are grouped.

## Current state

### 1. Run rejections never reach the log

`src-tauri/src/run.rs` — `run_project` rejects in four places and every one
returns before anything is written to the project's ring buffer:

- line ~496: spawn failure (`"{not_found} ({e})\nPATH searched: {path_searched}"`)
- line ~679: `"{} can't run: the folder {} no longer exists."`
- line ~695: the §9 step 1 port pre-check — the branch that produces
  `"Port {} is in use by {owner} — is this project running elsewhere?"`, naming
  the owning process and PID
- line ~720: the §6 guard rejection (`rejection.for_project(&project.name)`)

`src/store.ts` shows the consequence: `toast` is a single `string | null` that
`setToast` overwrites, and `src/App.tsx` renders exactly one. So the most
actionable sentence the app ever produces exists in one overwritable slot, is
destroyed by the next toast from *any* project, and appears in no log.

SPEC.md §11: *"Errors always say what happened and what to do next."*

The exemplar to follow is already in the file — `process::append_system(app,
&project.id, ...)` is used throughout `run.rs` for exactly this purpose.

### 2. Search unmounts running projects

`src/store.ts`:

```ts
export function filterProjects(projects: ProjectView[], search: string): ProjectView[] {
  const q = search.trim().toLowerCase();
  if (q === "") return projects;
  return projects.filter((p) => p.name.toLowerCase().includes(q));
}
```

`src/App.tsx` passes `filterProjects(projects, search)` straight to
`<ProjectGrid>`. A running project whose name does not match the query is
removed from the DOM, so its `PhaseStrip` `seen` set and its uptime clock both
reset — and the user cannot see that it is still running at all.

### 3. `pathExists` goes stale

`src-tauri/src/commands.rs:69` computes `path_exists` inside `to_view`, reached
only via `get_projects` / `add_project` / `update_project`. `src/App.tsx:115`
calls `loadRegistry()` once on mount; otherwise it runs only after add/update/
remove (`src/store.ts:334`, `:374`).

So a folder moved while the app is open keeps an enabled Run button and no
warning badge until restart. SPEC.md §5 requires the check *"at startup, on
registry change, **and when Run is clicked**"*, and §12's "Project path
deleted/moved" row promises *"Card warning state (`pathExists`), Run disabled,
Edit/Remove offered"*. The backend is honest (`run.rs:~679` re-checks and
rejects); the card is not.

### 4. Notes are invisible

`src/components/ProjectCard.tsx` renders `MENU_ITEMS` identically whether or not
a project has notes, and the card body never reads `project.notes`. A note
written today is unfindable among twenty projects.

### 5. No aggregate view; the port pill is inert

`src/App.tsx`'s header is title + search + Add + gear — nothing says how many
projects are running. `src/components/ProjectCard.tsx:233` renders
`:{project.port}` as plain text, where it is the natural click target for the
browser.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | 98 pass, 3 ignored |
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run verify`, `npm run build`, or `npm run test:acceptance` —
a 600 s no-output watchdog has killed executor runs here. Keep every Write/Edit
under ~60 lines and commit after each. Your reviewer runs the full suite.

## Scope

**In scope**:
- `src-tauri/src/run.rs` (append_system before the four rejections — nothing else)
- `src/store.ts` (running-count selector; leave `filterProjects` matching name only)
- `src/App.tsx` (running count in the header; pass running projects through the filter)
- `src/components/ProjectCard.tsx` (notes indicator; port pill as a browser affordance)

**Out of scope** (do NOT touch):
- The §6 state machine, any kill path, `process.rs`, `commands.rs`.
- The persisted schema — no new `Project` field. The notes indicator reads the
  field that already exists.
- **Extending search to match notes text.** §5 and §11 both say notes are
  "never parsed or acted on"; searching them would mean the app reads them.
  That needs a maintainer ruling and is deliberately excluded here.
- Log-buffer retention across runs — §8 mandates clearing at each Run, so that
  needs a spec amendment. Not this plan.
- Any new §7 command. Everything here uses existing commands.
- Card *contents* beyond the two changes named in steps 4 and 5 — §11 fixes the
  element list.

## Git workflow

- One commit per fix: `Visibility: <what>`.

## Steps

### Step 1: Log every Run rejection before returning it

In `src-tauri/src/run.rs`'s `run_project`, add a
`process::append_system(app, &project.id, <the same message>).await;` immediately
before each of the four `return Err(...)` sites listed in "Current state".

Notes:
- The site at ~line 496 is inside the spawn-failure branch and may already log;
  check before adding a duplicate line.
- For the "project not found" case, if no `project` is in scope there is no
  buffer to write to — skip it and say so in your report.
- Keep the message identical to the one returned, so the log and the toast agree.

**Verify**: `cargo check` → exit 0; `cargo test` → 98 pass. Then
`grep -c "append_system" src-tauri/src/run.rs` is higher than before (record
both numbers in your report).

### Step 2: Keep running projects visible under an active search

In `src/App.tsx`, change what is passed to `<ProjectGrid>` so that a project is
shown if it matches the search **or** is currently in a non-idle status
(anything other than `stopped` and `crashed`).

Implement it as a small exported pure function in `src/store.ts` beside
`filterProjects` — e.g. `visibleProjects(projects, search)` — so the rule is
testable and in one place. **Do not change `filterProjects` itself**; it is
correct and its name means what it says.

Order must still be preserved (SPEC.md §11: no automatic re-sorting) — build the
result with a single `filter` over the original array, never by concatenating
two lists.

Update the empty-results branch so it only shows when nothing at all is visible.

**Verify**: `npm run typecheck` → exit 0.

### Step 3: Refresh `pathExists` when it matters

In `src/App.tsx`, call the existing `loadRegistry()` on window focus — the
natural "user came back from the browser" moment, which is also §2 step 4.
Use a `useEffect` with a `window.addEventListener("focus", ...)` and a matching
cleanup.

Also call it after a rejected Run: in `src/store.ts`'s `startProject`, on the
catch path, `await loadRegistry()` after `setToast(...)` so the card picks up
the warning state that caused the rejection.

Do not poll on a timer — focus and rejection are the two moments §5 cares about
that are not already covered.

**Verify**: `npm run typecheck` → exit 0.

### Step 4: Show that a project has notes

In `src/components/ProjectCard.tsx`, when `project.notes` is non-empty, mark the
overflow-menu trigger — a small `text-muted` dot beside the `⋯`, or the menu
label reading `Notes ·`. Keep it quiet; §11's card list stays closed, so this
must read as a property of the existing control, not a new element.

Update the trigger's `aria-label` (currently `` `Actions for ${project.name}` ``)
to mention that notes exist, so it is not a visual-only signal.

**Verify**: `npm run typecheck` → exit 0.

### Step 5: Running count, and make the port pill open the browser

Two small changes:

- `src/App.tsx`: show a quiet count in the header when at least one project is
  running — e.g. `2 running` in `text-muted`. Derive it in `src/store.ts` as an
  exported pure helper (`runningCount(projects)`), counting status `running`
  only. Render nothing when the count is zero.
- `src/components/ProjectCard.tsx:233`: make the `:{project.port}` pill the
  click target for `openInBrowserAction(project.id)` when the project is
  `running`. Give it a `title` explaining what it does and an `aria-label`.
  When not running it stays inert text — do not render a dead-looking button.

This adds no card element: it makes an existing one (the status pill's port)
actionable, which §11's amended Card contents bullet permits ("their visual
treatment ... is not [fixed]").

**Verify**: `npm run typecheck` → exit 0; `cargo check` → exit 0.

### Step 6: Gates and commit

**Verify**: `cargo test` → 98 pass, 3 ignored; `npm run typecheck` → exit 0;
`git status --short` shows only in-scope files.

## Test plan

Rust: no new tests — step 1 adds log lines, not logic. The existing 98 must
still pass.

There is no JS test runner (SPEC.md §4's dependency rule), so `visibleProjects`
and `runningCount` are written as exported pure functions ready for one if it is
ever added.

Manual checks for the reviewer/maintainer (a subagent cannot drive the GUI):
- Register two projects on the same port via hand-edited JSON, Run the second →
  the toast names the owner **and** the message is in that project's log panel.
- Run a project, then type a search matching a different project → the running
  one stays visible, its uptime does not reset.
- Move a project's folder while the app is open, switch to another window and
  back → the card shows the warning badge and Run is disabled.
- Add a note to a project → the card's `⋯` shows the indicator.
- With one project running, the header shows `1 running`; clicking the port pill
  opens the browser.

## Done criteria

- [ ] `cargo test` → 98 passed, 3 ignored; `npm run typecheck` → exit 0
- [ ] Each of the four Run rejections logs before returning (report the
      `grep -c "append_system"` before/after counts)
- [ ] `visibleProjects` and `runningCount` are exported pure functions in `store.ts`
- [ ] `filterProjects` is unchanged
- [ ] `grep -rnE "#[0-9A-Fa-f]{6}" src/components/ src/App.tsx` → no matches
- [ ] No new `Project` field, no new §7 command, no state-machine change
- [ ] `plans/README.md` status row for 022 updated

## STOP conditions

Stop and report back if:

- Keeping running projects visible would require sorting or reordering — §11
  forbids automatic re-sorting; report instead.
- A fix appears to need a new command or a new persisted field. None does.
- Making the port pill clickable requires restructuring the status pill's
  markup enough to change the card's element list — report rather than
  redesigning the card.
- `cargo test` drops below 98 passing.

## Maintenance notes

- Step 2's rule ("visible if matching **or** non-idle") is a judgement about
  what a search is for. If folders or grouping ever land, revisit it — the same
  question will arise about whether a running project in a collapsed group
  stays visible.
- Two of these five were regressions introduced by earlier plans and missed in
  review: the search unmount (plan 017) and the unlogged rejections (present
  across four plans that touched `run_project`). Both are the same class —
  reviewing a diff for what it *does* rather than what it *hides*.
- Deliberately not addressed: log retention across runs (needs a §8 amendment)
  and searching note text (§5/§11 say notes are never read). Both need a
  maintainer decision first.
