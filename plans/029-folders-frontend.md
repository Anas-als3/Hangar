# Plan 029: Folders, frontend half — the tile, the band, and "Move to folder…"

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "folderId" src/types.ts`
> This must find the field. If it does not, plan 028 has not merged yet —
> **STOP**, nothing here will compile.

## Status

- **Priority**: P2 — maintainer-requested feature
- **Effort**: L — the largest single frontend change since M6
- **Risk**: MED — no gate in this repo renders a component, and this plan adds
  a whole new tile type
- **Depends on**: plan 028 (the two fields must exist in `types.ts`), plan 027
  (DONE — the phase strip must survive a card unmounting into a closed folder)
- **Category**: feature
- **Planned at**: commit `826cd21`, 2026-08-10

## Why this matters

The maintainer asked for iOS-style folders. This plan builds the whole feature
**except the drag gesture** (plan 030) — the folder tile, the inline band, the
menu route in and out, and search flattening. It is deliberately a complete,
usable feature on its own: menu-driven folders that work with a mouse and a
keyboard, shipped and verifiable before any webview drag code exists.

**Read SPEC.md §11's "Folders" and "Opening a folder" bullets, and the amended
"Card contents" bullet, before you start.** They were written for this plan and
they are the authority. Where this plan and §11 disagree, §11 wins and you
should report the discrepancy.

## The rule that governs everything here

> The grid is one walk of `projects` in array order: a project carrying a
> `folderId` is not drawn as its own tile, and the first time the walk reaches
> any member of a folder, that folder's tile is drawn in that position.

```
for p of projects (array order):
  p.folderId == null                     -> emit ProjectCard(p)
  p.folderId not yet emitted             -> emit FolderTile(p.folderId); mark emitted
  otherwise                              -> skip (it renders inside its tile)
```

**The array is never rewritten.** Not on create, open, rename, ungroup, move-in
or move-out. Delete every folder and the grid returns card-for-card to the order
it has today. `grid-auto-flow: dense` is forbidden by name in §11 — do not add
it, and do not add any comparator, `sort`, or `concat` of two filtered lists.

## Current state

`src/components/ProjectGrid.tsx` — the whole file is 40 lines; the map and the
trailing tile:

```tsx
export function ProjectGrid({ projects }: { projects: ProjectView[] }) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-3">
      {projects.map((project) => (
        <ProjectCard key={project.id} project={project} />
      ))}
      <AddTile />
    </div>
  );
}
```

`src/App.tsx:178-186` — the grid is fed `visibleProjects(projects, search)` and
an empty-state gate consumes the **same** call:

```tsx
        ) : visibleProjects(projects, search).length === 0 ? (
          <p className="text-sm text-muted">No projects match &quot;{search.trim()}&quot;.</p>
        ) : (
          <ProjectGrid projects={visibleProjects(projects, search)} />
        )}
```

`src/store.ts:142-154` — `visibleProjects`, which must be reused **unchanged**:

```ts
export function visibleProjects(projects: ProjectView[], search: string): ProjectView[] {
  const q = search.trim().toLowerCase();
  if (q === "") return projects;
  return projects.filter(
    (p) => p.name.toLowerCase().includes(q) || (p.status !== "stopped" && p.status !== "crashed"),
  );
}
```

`src/components/ProjectCard.tsx:117-127` — `MENU_ITEMS`, the array to extend.

`src/components/ProjectCard.tsx:44-65` — `STATUS_LABEL` and `STATUS_TONE`. The
folder tile's dots reuse `STATUS_TONE` with `bg-current`; **lift it to a shared
module rather than duplicating it**. Same for `lastRunLabel`
(`ProjectCard.tsx:76-87`).

`src/components/NotesPanel.tsx` and `src/components/AddEditDialog.tsx` — read
both before writing any new surface. `AddEditDialog`'s shell (its backdrop,
`hangar-dialog-in`, focus handling, Esc) is the pattern the "Move to folder…"
dialog must follow; do not invent a second dialog idiom.

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
- `src/store.ts` — folder derivation, open/closed set, the folder actions
- `src/components/ProjectGrid.tsx` — the one walk
- `src/components/FolderTile.tsx` — new
- `src/components/MoveToFolderDialog.tsx` — new
- `src/components/ProjectCard.tsx` — **one** new menu item, nothing else
- `src/App.tsx` — mount the new dialog; the neutral toast variant
- `src/index.css` — only if a token is genuinely missing (it should not be)
- A small shared module for `STATUS_TONE` / `lastRunLabel` lifted out of
  `ProjectCard.tsx`

**Out of scope** (do NOT touch):
- **Anything under `src-tauri/`.** Plan 028 owns the backend and may still be in
  flight. No Rust, no schema, no command.
- **`src/types.ts`.** Plan 028 owns it. The fields are already there.
- **Any drag code.** No `pointerdown`, no `draggable`, no `dragstart`, no drop
  targets, no ghost. That is plan 030 and it is deliberately separate. If you
  add drag handling here you have exceeded scope — STOP.
- `visibleProjects`, `filterProjects`, `runningCount` — reuse, never edit.
- The phase strip, the status pill, the port button, the stack badge, the
  libraries line, the Run/Stop button, the log panel, the notes panel, the
  settings dialog.
- Persisting open/closed state. It is ephemeral view state and must never reach
  `projects.json`.
- Folder-name search. §11 says search **dissolves** folders; searching their
  names is explicitly deferred.
- Any new dependency. No drag library, no id library — use
  `crypto.randomUUID()` (available in WKWebView) or a timestamp+counter in the
  style of `registry.rs`'s id comment.

## Git workflow

- One commit per step: `Folders UI: <what>`.

## Steps

### Step 1: Lift the shared bits out of `ProjectCard`

Move `STATUS_TONE`, `STATUS_LABEL` and `lastRunLabel` into a new
`src/components/status.ts` (or `src/status.ts` — match where the repo puts
non-component modules) and import them back into `ProjectCard.tsx`. **No
behaviour change, no value change.** This exists so the folder tile cannot fork
the palette.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0;
`git diff --stat` shows `ProjectCard.tsx` losing exactly those definitions and
gaining an import.

### Step 2: Derivation in the store

Add to `src/store.ts`, as pure exported functions beside `visibleProjects`:

1. `gridItems(projects: ProjectView[], search: string)` — the one walk above,
   returning a discriminated union of `{kind:"project", project}` and
   `{kind:"folder", id, name, members}`. **When `search.trim() !== ""` it
   returns every project from `visibleProjects` as `kind:"project"`** — folders
   dissolve, per §11. Members are in array order. The folder's `name` comes from
   its **earliest member** (§5: that is the tiebreak when members disagree).
2. `openFolders: Set<string>` in `HangarState`, plus `toggleFolder(id)`.
   Ephemeral — initialise to an empty set, and **`loadRegistry()` must not
   touch it**. `loadRegistry` fires on every window focus and after every
   add/update/remove; resetting the set there would collapse the folder you just
   opened every time you come back from the browser.
3. `folderSummary(members)` → the counts line. Fragments in **fixed severity
   order**: `n stop-failed`, `n crashed`, `n running`, `n in progress` (the
   `updating|installing|starting|stopping` bucket), joined by ` · `. When every
   member is `stopped`, return the most recently run member's `lastRunLabel`
   instead. Omit zero-count fragments.
4. `moveToFolder(projectId, target)` where target is an existing folder id, a
   new folder name, or `null` (out). It must:
   - read the project fresh via `findProject(projectId)` **at call time**, never
     from a value captured earlier — copy `saveNotesAction`'s shape exactly;
   - call `updateProject` with the record plus the changed folder fields;
   - `loadRegistry()` after.
5. `renameFolder(folderId, name)` — N `updateProject` calls, one per member.
6. `ungroupFolder(folderId)` — N calls clearing both fields.

**A folder auto-expands and cannot be collapsed while any member is
`stop-failed`.** Implement this as a derived predicate at render time, not by
mutating `openFolders` — that keeps it impossible to get stuck.

**Verify**: `npm run typecheck` → exit 0.

### Step 3: The folder tile

New `src/components/FolderTile.tsx`, following §11's Folders bullet exactly:

- Root is an `<article>`, **not a `<button>`** — it contains the `⋯` button and
  (when renaming) an `<input>`, and a button may not contain either. Open/close
  is a stretched `absolute inset-0` button with `aria-expanded` /
  `aria-controls` and an `sr-only` label; `⋯` and the rename input are siblings
  at `relative z-10`. `ProjectCard.tsx:167-233` is already this shape — copy it.
- Contents, and nothing else: the `›`/`⌄` glyph + name (Space Grotesk, `truncate`
  + `title`), the member count, the dot row, the counts line.
- Dots: one `size-1.5 rounded-full bg-current` per member in array order, inside
  a span carrying that member's `STATUS_TONE` class. A member with
  `pathExists === false` renders as a hollow ring (`border border-current
  bg-transparent`) **while keeping its status colour** — shape carries the
  warning, colour still carries the status. Cap at 8, then `+n`. The row is
  `aria-hidden`, with an `sr-only` list of member names and statuses beside it.
- **No Run button, no status pill, no port, no stack badge, no libraries line,
  no phase strip.** §11 lists these as absent by design.
- Marked by shape: `border-white/10` (cards use `border-white/5`), two 1px ghost
  edges above the top border, and the missing Run button. **No emoji, no new
  colour.**
- Its `⋯` menu is exactly `Rename · Ungroup`. Ungroup confirms with
  `Ungroup "<name>"? The <n> projects stay in your library.` The word **Remove
  must not appear** — in this app it means "destroy this project, cannot be
  undone".
- Rename is inline in the **open band's header**, never on the closed tile
  (that would collide with the open gesture). Enter commits, Esc cancels, blur
  commits, an empty/whitespace commit reverts to the previous name.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 4: The grid walk and the band

Rewrite `ProjectGrid.tsx` to map `gridItems(...)` instead of `projects`:

- A `kind:"project"` item renders `<ProjectCard>` exactly as today.
- A `kind:"folder"` item renders `<FolderTile>`, and — when open — a **sibling**
  `<section>` immediately after it with `grid-column: 1 / -1`,
  `border-t border-b border-white/10`, `bg-white/[0.02]`, `py-3` and **zero
  horizontal padding**, nesting a grid on the *same* `minmax(14rem,1fr)` track.
  Zero horizontal padding is load-bearing: with `p-3` the nested auto-fill drops
  a column at boundary widths and cards render *wider* inside folders than
  outside.
- Member cards are the **unmodified `<ProjectCard>`**.
- Esc closes the band via the band's own `onKeyDown`, **not** a `document`
  listener — a document listener collides with `ProjectCard`'s menu handler
  (`ProjectCard.tsx:140-147`) and the search box's clear-on-Escape
  (`App.tsx:104-106`). One keypress must never fire two unrelated state changes.
- The `<AddTile>` stays last, still outside the map.
- `App.tsx`'s empty-state gate must keep consuming the same source as the grid,
  so the two can never disagree.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 5: "Move to folder…" and the neutral toast

1. Add **one** item to `MENU_ITEMS` in `ProjectCard.tsx`: `Move to folder…`,
   **after Notes and before Edit** (§11 fixes the order). Change nothing else in
   that file.
2. New `src/components/MoveToFolderDialog.tsx` on `AddEditDialog`'s shell: a
   radio list of existing folders, a `New folder…` row with a text field, and a
   **`Not in a folder`** row. That last row is the only non-mouse way *out* of a
   folder and the closest thing this feature has to an undo — it is always
   present, never conditionally rendered.
3. Mount it in `App.tsx` beside the other dialogs.
4. `App.tsx:76-93`'s `Toast` is styled as an **error**
   (`border-status-danger/40`). Announcing "Moved Example App to Client Work" in
   red is wrong. Add a neutral variant — a `tone` prop defaulting to the current
   error styling so **every existing call site is unchanged** — and use it for
   the move confirmation.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 6: Gates and a structural self-check

Report each:

- `grep -rn "grid-auto-flow\|\.sort(\|dense" src/` → no match in the grid path.
- `grep -rn "pointerdown\|dragstart\|draggable" src/` → **no matches at all**.
  Drag is plan 030.
- `grep -rn "openFolders" src/store.ts` → present, and **not** written by
  `loadRegistry`.
- `grep -rn "folderId\|folderName" src-tauri/` → only plan 028's work; nothing
  new from you.
- `grep -rnE "#[0-9A-Fa-f]{6}" src/components/FolderTile.tsx src/components/MoveToFolderDialog.tsx`
  → no matches.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0;
`git status --short` shows only in-scope files.

## Test plan

There is no JS test runner (SPEC.md §4's dependency rule) and **no gate in this
repo renders a component** — a full-screen modal shipped on 2026-08-09 with
every check green. Reason carefully and show the JSX in your report.

Manual checks for the reviewer/maintainer:

- Two projects, `Move to folder… → New folder…`, name it. Both vanish into one
  tile at the **first** one's grid position; the second project's old position
  closes up and every other card keeps its place.
- Open the folder. The member cards are pixel-identical to cards outside it.
- Run a project **inside a closed folder**. The header count increments, the
  tile shows a green dot and "1 running", and Stop is one click after opening.
- Its phase strip is intact after opening — plan 027 is what makes this true.
- Force a `stop-failed` member: the folder must open itself and refuse to close.
- Search for something that matches nothing: folders disappear entirely and
  matching projects render flat.
- Search while a folder member is running: it stays visible (plan 022's rule).
- `Move to folder… → Not in a folder` on the last two members: the folder
  disappears with no leftover, and `projects.json` has no folder keys.
- Rename, then quit and relaunch: the name persists, and every folder is closed.

## Done criteria

- [ ] `npm run typecheck` exits 0
- [ ] `npm run build` exits 0
- [ ] No `sort`, no `concat` of two filtered lists, no `grid-auto-flow: dense`
- [ ] No drag code anywhere in `src/`
- [ ] `openFolders` is never persisted and never reset by `loadRegistry`
- [ ] `STATUS_TONE` and `lastRunLabel` exist in exactly one module
- [ ] The folder tile is an `<article>`, and its menu says Ungroup, never Remove
- [ ] Nothing under `src-tauri/` or in `src/types.ts` modified
- [ ] `plans/README.md` status row for 029 updated

## STOP conditions

Stop and report back if:

- `grep -n "folderId" src/types.ts` finds nothing — plan 028 has not merged.
- Making the folder tile fit needs a change to the grid track, the card padding,
  or the removal of any card element. §11 fixes both; report instead.
- You want to persist which folders are open. It is view state; §5 lists exactly
  what reaches `projects.json`.
- You want to add drag handling "while you're in there". Plan 030 owns it, and
  it depends on a runtime probe that has not been run.
- A folder needs a Run button, a status pill, or its own state machine. §6's
  status vocabulary belongs to projects; §11 lists these as absent by design.

## Maintenance notes

- The grid walk is the load-bearing invariant. Any future feature that wants to
  change what order tiles appear in is a §11 amendment, not a code change.
- `moveToFolder` reading the project fresh at call time is the same defence
  `saveNotesAction` uses. Anything that writes a project from a value captured
  before an `await` will roll back whatever a run wrote in between.
- The tile's `min-h` should match a **resting card measured in a real window**,
  not an estimate — plan 026's libraries line changed that height after this
  design was drafted. A folder tile visibly shorter than its row-mates is the
  fastest way to make this look bolted on.
- When plan 030 lands, the only tile-side change should be a `data-` attribute
  for hit-testing. If it needs more, this plan's structure was wrong.
