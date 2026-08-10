# Plan 033: Three folder-UI defects found by review

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "closeFolder" src/components/ProjectGrid.tsx`
> Must exist. If it does not, plan 029 has not merged — **STOP**.

## Status

- **Priority**: P2 — one destructive-feeling interaction, one visual break, one
  stale-UI-after-failure
- **Effort**: S
- **Risk**: LOW — three localised changes, no backend, no schema
- **Depends on**: plans 029 and 030 (both must be merged — this touches the same
  files)
- **Category**: bug
- **Planned at**: 2026-08-10

## Why this matters

All three were found by a fresh-context reviewer reading the merged folders
diff. None is catchable by any gate in this repo: `typecheck`, `build` and
`cargo test` all pass with every one of them present.

## Defect 1 — Esc inside an open folder closes the folder as well as the menu

`src/components/ProjectGrid.tsx:51-56` gates the band's Esc on focus, as
SPEC.md §11 requires:

```tsx
onKeyDown={(e) => { if (e.key === "Escape") closeFolder(folderId); }}
```

But `src/components/ProjectCard.tsx:114-118` closes its overflow menu from a
**`document`** listener, and every member card lives **inside** the band. "Focus
is inside the band" and "a card menu is open" are the same state, not exclusive
ones.

**Failure scenario.** Folder "Client Work" is open. The user tabs to a member
card's `⋯` and presses Enter, then presses Esc to dismiss the menu. React's
delegated handler fires the band's `onKeyDown` first → `closeFolder` → the
folder collapses and every member card unmounts. The document listener then
fires `setMenuOpen(false)` on a component that no longer exists. **The user
asked to dismiss a menu and lost the whole folder.**

The same double-fire happens with the log panel (`LogPanel.tsx`) and the notes
panel (`NotesPanel.tsx`), which are also `document`-scoped: Esc closes the panel
*and* collapses whatever folder the focus happened to be in.

Verified **not** affected, so do not touch them: the search box's
clear-on-Escape (a React handler on an input in the header, outside any band —
and an active search dissolves bands anyway), and the folder rename input (the
tile is a *sibling* of the band, not a descendant).

## Defect 2 — the folder tile's `⋯` menu paints underneath the tile's own text

In `src/components/FolderTile.tsx` all four direct children of the `<article>`
carry `relative z-10`: the `<header>` (which contains the menu), the "N
projects" `<p>`, the `FolderDots` wrapper, and the summary `<p>`.

`position: relative` **plus** `z-index: 10` creates a stacking context. Four
siblings at the same z-index paint in **DOM order**, so the header — and the
entire menu subtree trapped inside its context — paints *first*, beneath the
three siblings that follow it.

`src/components/ProjectCard.tsx` gets this right by accident of shape: its
`<header>` is **non-positioned**, so the menu's `z-10` escapes into the card's
stacking context and correctly overlays everything.

**Failure scenario.** A folder with 6+ members at the default 14 rem track. The
menu is `absolute right-0 mt-1 w-40`, so it covers roughly the same band as the
count line and the dot row. Open it: status dots and the tail of "8 projects"
render **on top of** the menu's opaque panel, across the "Rename" row.

The `pointer-events-none` added in `780a595` kept the menu *clickable*, which is
why this survived review — pointer-events and paint order are independent.

## Defect 3 — a partial rename/ungroup leaves the grid showing the old state

`src/store.ts`'s `renameFolder` and `ungroupFolder` each loop N sequential
`updateProject` calls and then `await loadRegistry()` — **inside the `try`**.
The `catch` only toasts. So a mid-sequence rejection aborts the loop, the store
is never refreshed, and the grid keeps rendering the *old* folder name or the
*intact* folder while disk holds a partial write.

**Failure scenario.** Folder with members [A, B, C]; the user picks Ungroup and
confirms. A is written. B is rejected. C is never attempted. The user sees an
error toast and a tile that still says "3 projects", with nothing indicating A
already left. Only the next window focus reveals it.

The data model does not corrupt — §5's "earliest member supplies the name"
tiebreak holds, and re-issuing either action is idempotent for members already
written. The defect is that the user is shown a stale grid at the moment they
most need an accurate one.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |
| Bundle | `npm run build` | exit 0 |

**Do not run** `npm run verify`, `npm run build:app`, `cargo` anything, or
`npm run test:acceptance` — a 600 s no-output watchdog has killed executor runs
here. This plan touches no Rust. Keep every Write/Edit under ~60 lines and
commit after each.

## Scope

**In scope**:
- `src/components/ProjectCard.tsx` — the menu's Esc handling only
- `src/components/ProjectGrid.tsx` — the band's Esc guard only
- `src/components/FolderTile.tsx` — stacking classes only
- `src/store.ts` — `renameFolder` / `ungroupFolder` reload placement only

**Out of scope** (do NOT touch):
- Anything under `src-tauri/`, and `src/types.ts`.
- The drag subsystem (`src/cardDrag.ts`, `src/dragGeometry.ts`) — plan 030 owns
  it and it is already merged. Do not refactor it.
- `MoveToFolderDialog.tsx`, `LogPanel.tsx`, `NotesPanel.tsx`, `App.tsx`. Defect
  1's fix must handle them **without editing them** — see step 2.
- `gridItems`, `folderSummary`, `moveToFolder`, `visibleProjects`.
- The search box's Escape handling. Verified correct.
- Any new dependency.

## Git workflow

- One commit per step: `Folder fixes: <what>`.

## Steps

### Step 1: Scope the card menu's Esc to the card menu

In `ProjectCard.tsx`, replace the `document` **keydown** listener with a React
`onKeyDown` on the wrapper `<div ref={menuRef}>` that already contains both the
`⋯` button and the menu. The handler closes the menu **and calls
`event.stopPropagation()`** — both handlers are React synthetic handlers on the
same tree, so stopping propagation there genuinely prevents the band's handler
from firing.

Keep the `document` **mousedown** outside-click listener exactly as it is. Only
the keydown listener moves.

Behaviour change to state in your report: Esc now closes the menu only while
focus is inside the menu or on its trigger. That is standard menu behaviour, and
the click-outside handler still covers dismissing it any other way.

**Verify**: `npm run typecheck` → exit 0.

### Step 2: Make the band's Esc yield to any open overlay

Step 1 fixes the card menu. The log panel, the notes panel and the dialogs are
out of scope to edit, so the band must yield to them instead.

In `ProjectGrid.tsx`'s `OpenBand`, read the store and ignore Esc when any
overlay is open — `openLogsFor`, `notesFor`, or `dialog` is non-null. One
keypress must never fire two unrelated state changes.

Add a comment naming the three, and why: those surfaces own Esc while they are
open, and each closes itself from a `document` listener the band cannot see.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 3: Fix the folder tile's paint order

In `FolderTile.tsx`, remove `relative z-10` from the three display-only
siblings — the "N projects" `<p>`, the `FolderDots` root `<div>`, and the
summary `<p>` — keeping their `pointer-events-none`.

Leave the `<header>` at `relative z-10`: it must stay above the stretched
open/close button so the `⋯` button and the rename input remain hit-testable.

Why this is safe for the three siblings: the stretched button is transparent, so
text painting beneath it is still fully visible, and all three are
`pointer-events-none` already, so hit-testing passes through them regardless.

Add a comment explaining the constraint, because the next person will be tempted
to "tidy" the classes back to matching: **the header is the only child that may
carry `z-10`, or the menu it contains paints beneath its own siblings.**

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 4: Refresh after a partial folder write

In `store.ts`, move the `await loadRegistry()` in `renameFolder` and
`ungroupFolder` so it runs on **both** paths — a `finally`, or an explicit call
in the `catch` as well. The user must never be shown a folder that no longer
matches disk.

Do not add retry logic, rollback, or a progress indicator. §5's recovery is
already specified and the next rename repairs a partial one; the only bug here
is the stale view.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 5: Self-check

Report each:

- `grep -n "addEventListener(\"keydown\"" src/components/ProjectCard.tsx` → no
  match (the mousedown listener stays).
- `grep -n "z-10" src/components/FolderTile.tsx` → the header and the menu, and
  nothing else among the `<article>`'s direct children.
- `grep -n "loadRegistry" src/store.ts` → the calls in `renameFolder` and
  `ungroupFolder` are reachable on the failure path.
- `git status --short` → only the four in-scope files.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

## Test plan

No JS test runner for component behaviour (SPEC.md §4). Manual checks for the
maintainer:

- Open a folder, open a member card's `⋯`, press Esc → the **menu** closes, the
  folder stays open. Press Esc again with focus still in the band → the folder
  closes.
- Open a folder, open a member's log panel, press Esc → the **panel** closes, the
  folder stays open.
- Open a folder tile's `⋯` on a folder with 6+ members → the menu renders fully
  opaque, with no dots or text painted across it.
- Rename a folder while one member is running such that the write fails → the
  toast appears **and** the grid shows the actual current state.

## Done criteria

- [ ] `npm run typecheck` exits 0
- [ ] `npm run build` exits 0
- [ ] `ProjectCard.tsx` has no `document` keydown listener; its menu Esc calls `stopPropagation`
- [ ] `OpenBand` ignores Esc while a log panel, notes panel or dialog is open
- [ ] Only the `<header>` among `FolderTile`'s direct children carries `z-10`
- [ ] `renameFolder` and `ungroupFolder` reload on the failure path
- [ ] Nothing under `src-tauri/`, no drag file, no dialog file modified
- [ ] `plans/README.md` status row for 033 updated

## STOP conditions

Stop and report back if:

- Fixing defect 1 seems to need edits to `LogPanel.tsx`, `NotesPanel.tsx` or
  `App.tsx`. It does not — step 2 has the band yield instead.
- `stopPropagation` does not prevent the band's handler. That would mean the two
  handlers are not both React synthetic handlers on the same tree; report what
  you found rather than reaching for a `document` listener.
- Removing `z-10` from the three siblings visibly changes anything other than
  the menu's overlap. It should not; they are transparent-background text.

## Maintenance notes

- The general rule this repo keeps rediscovering: **`document`-scoped keyboard
  listeners do not compose.** Every surface that owns Esc must either scope the
  listener to its own subtree or explicitly yield to the surfaces above it.
  There are now four Esc owners (card menu, band, panels, dialogs); a fifth
  should be a design conversation, not a fifth listener.
- `FolderTile`'s stacking is fragile by construction: one stretched button
  underneath, one header that must sit above it, and display-only content that
  must not create competing contexts. The comment added in step 3 is the only
  thing standing between the next editor and re-breaking it.
