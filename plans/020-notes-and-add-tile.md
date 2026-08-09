# Plan 020: Per-project notes slide-over, and an add tile at the end of the grid

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat 4a8f8fb..HEAD -- src-tauri/src/registry.rs src/types.ts src/store.ts src/App.tsx src/components/`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: LOW-MED — the notes field touches the persisted schema and the
  frozen §7 wire contract's test guard, both of which have specific rules.
- **Depends on**: none (SPEC.md §5 and §11 were amended 2026-08-09 to describe
  both features; this plan implements those amendments)
- **Category**: direction (maintainer-requested)
- **Planned at**: commit `4a8f8fb`, 2026-08-09

## Why this matters

Two maintainer requests:

1. **Notes** — a free-text scratchpad per project, for recording ideas after
   testing something. §5 now carries `notes?: string` and §11 describes a
   slide-over opened from the overflow menu.
2. **An add tile** — a `+` affordance at the end of the card grid, so adding a
   project does not mean travelling to the header button every time.

Neither is on §3's OUT list. Both amendments are already applied to `SPEC.md`;
this plan is the implementation.

## Two things that are already decided — do not re-derive them

**The notes field does NOT need §16's versioned storage wrapper.** That parked
entry is for *incompatible* schema changes. `Project` already uses
`#[serde(default, skip_serializing_if = "Option::is_none")]` on every optional
field, so an added optional field is compatible in both directions: old
`projects.json` files load (field absent → `None`), and files written with notes
still open in an older build (§4: "Unknown JSON fields are ignored, never a
fatal error"). Do not add a schema-version wrapper.

**No new §7 command.** §7 is FROZEN. `update_project(project: Project)` already
carries the whole record, so saving notes is an `update_project` call with the
notes field set. Do not add `set_notes` or anything like it.

## Current state

- `src-tauri/src/registry.rs` — the `Project` struct ends:

```rust
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
}
```

  `NewProject` is a separate struct in the same file with the same field style.
- `src-tauri/src/registry.rs` also holds the §7 wire-contract guard,
  `every_wire_key_the_backend_emits_appears_in_types_ts`, which builds a
  fully-populated `ProjectView` from a `sample()` helper (~line 664) and asserts
  every emitted JSON key is declared as a property in `src/types.ts`. **A new
  field must be set in `sample()`**, or the guard silently never checks it —
  its own maintenance note says so.
- `src/types.ts` mirrors the Rust shapes by hand; `Project` there needs the same
  optional field. `NewProject` is `Omit<Project, "id" | "lastLockfileHash" | "lastRunAt">`.
- `src/components/LogPanel.tsx` (181 lines) is the slide-over pattern to copy:
  `fixed inset-0 z-20 flex justify-end`, a backdrop with `hangar-fade-in`, the
  panel with `hangar-slide-in`, Esc closes, and it reads `openLogsFor` from the
  store.
- `src/store.ts` has `openLogsFor: string | null` plus `openLogs`/`closeLogs`,
  and a `DialogState` union for the add/edit/settings dialogs. Follow whichever
  shape fits — notes are a slide-over like logs, not a modal dialog.
- `src/components/ProjectCard.tsx` has `MENU_ITEMS`, an array of
  `{ label, action }`. All five entries are wired. §11's amended menu order is:
  Open in browser · Open in editor · Show logs · **Notes** · Edit · Remove.
- `src/components/ProjectGrid.tsx` renders
  `grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-3` over
  `projects.map(...)`.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | 93 pass, 3 ignored |
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run verify`, `npm run build`, or `npm run test:acceptance` —
a 600 s no-output watchdog has killed executor runs on this repo. Keep every
Write/Edit under ~60 lines and commit after each. Your reviewer runs the full suite.

## Scope

**In scope**:
- `src-tauri/src/registry.rs` (the `notes` field on `Project` + `NewProject`, and
  the wire-guard `sample()`)
- `src/types.ts` (mirror the field)
- `src/store.ts` (notes panel open/close state + a save action)
- `src/components/NotesPanel.tsx` (**create**)
- `src/components/ProjectCard.tsx` (the `Notes` menu entry)
- `src/components/ProjectGrid.tsx` or `src/App.tsx` (the add tile)

**Out of scope** (do NOT touch):
- Any new §7 command. `update_project` carries notes.
- A schema-version wrapper (see "already decided" above).
- `src-tauri/src/run.rs`, `process.rs`, `commands.rs` — notes never affect
  running a project. Nothing may read the field except the UI that edits it.
- Card contents — §11 still fixes the list. Notes are reachable from the menu,
  and there is **no** indicator on the card.
- Motion tokens/durations from plan 018, the palette from plan 019.
- Any new dependency.

## Git workflow

- One commit per file: `Notes and add tile: <what>`.

## Steps

### Step 1: The persisted field

Add to `Project` in `src-tauri/src/registry.rs`, matching the existing style:

```rust
    /// SPEC.md §5: free-text scratchpad, user-owned. Nothing in the app parses
    /// or acts on it — it exists only to be shown and edited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
```

Add the same to `NewProject`. Mirror it in `src/types.ts` as `notes?: string;`
with a matching comment.

**Then set it in the wire guard's `sample()`** (~line 664 area) to a non-empty
value, so `every_wire_key_the_backend_emits_appears_in_types_ts` actually
covers the new key. If you skip this the guard passes while checking nothing —
which is exactly the failure its maintenance note warns about.

**Verify**: `cargo test` → all pass including the wire guard;
`npm run typecheck` → exit 0.

### Step 2: Store state and save action

Add notes-panel state to `src/store.ts` in the style of `openLogsFor`:
`notesFor: string | null`, with `openNotes(projectId)` / `closeNotes()`.

Add `saveNotesAction(project: Project, notes: string)` that calls the existing
`updateProject` API wrapper with the notes field replaced, then `loadRegistry()`
to refresh, and `setToast(...)` on rejection — same shape as the other actions.

**Verify**: `npm run typecheck` → exit 0.

### Step 3: The notes panel

Create `src/components/NotesPanel.tsx`, modelled closely on `LogPanel.tsx`:
same slide-over shell (`fixed inset-0 z-20 flex justify-end`), backdrop with
`hangar-fade-in`, panel with `hangar-slide-in`, Esc closes, returns `null` when
`notesFor` is null.

Contents: the project name as a heading, one large `<textarea>` filling the
panel, and a Close button. Autosave on blur and on a debounce (~800 ms) after
typing stops — do not add a Save button; §11 says "autosaved". Show a small,
quiet saved indicator using `text-muted`; do not animate it beyond what plan
018's utilities already provide.

Use only existing tokens — `bg-surface`, `bg-bg`, `text-text`, `text-muted`,
`border-white/10`, `font-display` for the heading, and `font-mono` only if it
genuinely suits (prose notes probably want the sans default). **No raw hex.**

**Verify**: `npm run typecheck` → exit 0.

### Step 4: Wire the menu entry and render the panel

In `ProjectCard.tsx`, add `{ label: "Notes", action: (id) => void openNotes(id) }`
to `MENU_ITEMS`, positioned per §11's amended order: after "Show logs", before
"Edit". Do not restructure the array.

Render `<NotesPanel />` in `src/App.tsx` alongside `<LogPanel />`.

**Verify**: `npm run typecheck` → exit 0.

### Step 5: The add tile

Add a `+` tile as the **last** item in the grid, after the mapped project cards,
so it appears at the end of the flow. It opens the add dialog
(`openAddDialog` already exists in the store).

Requirements:
- Same grid cell dimensions as a card so the layout stays even — a dashed
  `border-white/10` outline reads as an affordance rather than a project, and
  keeps it visually distinct from real cards.
- A `+` glyph and a short label; `aria-label` on the button.
- It must **not** be rendered when the registry is empty — the §11 first-run
  empty state ("No projects yet. Add your first one." + Add button) owns that
  case, and two competing add affordances there would be noise.
- **It is not a project card**: it must not be inside the `projects.map(...)`
  output, must not receive a project prop, and must not shift card ordering.
  §11's array-order rule is about projects; the tile trails them.
- Leave the header Add button alone — this is an addition, not a replacement.

**Verify**: `npm run typecheck` → exit 0.

### Step 6: Gates and commit

**Verify**: `cargo test` and `npm run typecheck` both pass; `git status --short`
shows only in-scope files.

## Test plan

Rust: the wire guard covers the new key once `sample()` sets it (step 1) — that
is the automated coverage. Add nothing else; notes have no backend behaviour to
test because nothing reads them.

There is no JS test runner (SPEC.md §4's dependency rule), and no gate renders a
component. Manual checks for the reviewer/maintainer:
- Open Notes on a project, type, close, reopen → the text is still there.
- Restart the app → notes persist (they are in `projects.json`).
- Edit a project's port via the Edit dialog → notes survive the update.
- Esc closes the panel; the backdrop click closes it.
- The `+` tile sits after the last card and opens the add dialog.
- With zero projects, the `+` tile is absent and the first-run empty state shows.
- Adding a project via the tile appends it, and the tile stays last.

## Done criteria

- [ ] `cargo test` passes (93 + any the guard now covers) and `npm run typecheck` exits 0
- [ ] `notes` appears in `Project` and `NewProject` (Rust), in `src/types.ts`, and in the wire guard's `sample()`
- [ ] `grep -rn "set_notes\|save_notes" src-tauri/src/` → no matches (§7 unchanged)
- [ ] `grep -rn "schemaVersion" src-tauri/src/` → no matches (no storage wrapper)
- [ ] `grep -rnE "#[0-9A-Fa-f]{6}" src/components/ src/App.tsx` → no matches
- [ ] Nothing in `run.rs`/`process.rs` reads `notes`
- [ ] `plans/README.md` status row for 020 updated

## STOP conditions

Stop and report back if:

- Saving notes appears to need a new command — it does not; `update_project`
  takes the whole `Project`.
- The wire-contract guard fails after adding the field — that means the Rust and
  TS shapes have genuinely diverged; report rather than editing either side to
  make it pass.
- The add tile cannot be placed without putting it inside `projects.map(...)` or
  changing card order — report the constraint instead.
- You find yourself wanting anything to *read* the notes field (search, tags,
  parsing) — §5 says user-owned and never acted on. That would be a new feature
  needing its own decision.

## Maintenance notes

- If project search (plan 017) is ever extended to match notes, that is a
  deliberate decision, not an obvious extension — §5 currently says the app
  never reads the field, and matching on hidden text makes results
  inexplicable, which is why 017 matches on name only.
- The wire guard only checks the samples it is fed. Any future §7 field needs
  adding to `sample()` in the same commit, or it ships unchecked.
- The add tile duplicates the header button deliberately. If the header button
  is ever removed, re-check the empty-state path, which relies on it.
