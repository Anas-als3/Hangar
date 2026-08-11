# Plan 037: Make `+N` a button that reveals the whole stack

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "stackHoverText\|libraries.length > 3" src/components/ProjectCard.tsx`
> Both must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `npm run typecheck`. **Your reviewer runs
> `npm run build` and `cargo check`.** This plan touches no Rust.

## Status

- **Priority**: P2 — maintainer-requested, twice
- **Effort**: S
- **Risk**: LOW-MED — no gate in this repo renders a component, and this adds
  an overlay to a card that already hosts a menu, a drag source and a folder band
- **Depends on**: SPEC.md §11 amendment (ratified 2026-08-10)
- **Category**: feature
- **Planned at**: commit after the §11 amendment, 2026-08-10

## Why this matters

The maintainer, verbatim: *"it shows the + part and with number but i cant see
what are they, their should be a clickable function on that + so it shows the
stack and apis"*.

A `title` tooltip shipped hours earlier for exactly this purpose. They did not
find it. **Treat "hover already does this" as refuted** — the request is the
evidence that the affordance failed.

Live registry: only the two `example-monorepo` cards render a `+N`, reading
`+5`. Hidden behind it: **Express · TypeScript · Zod · Playwright · Vitest**.
`OpenAI` and `Anthropic` are already in the visible three, so the "apis" half of
the ask is satisfied by backend ordering. Example App (`React`, `TypeScript`)
has no `+N` and gains no control.

**And the old fallback route was worse than nothing.** §11 used to say the full
list "remains in the Edit dialog". `handleEdit` (`ProjectCard.tsx:40-46`) calls
`stopIfRunningWithConfirm` (`store.ts:714-720`), which on a running project
prompts *"… is running. Stop it first?"* and, on confirm, **stops the dev
server**. The documented route to five dependency names offered to kill your
server. §11 was amended for this; read its "Card contents" bullet before
starting.

## Current state

`src/components/ProjectCard.tsx` — the libraries line as it stands:

```tsx
      {project.stack && project.stack.libraries.length > 0 && (
        <p
          className="flex items-baseline gap-1 text-xs text-muted"
          title={stackHoverText(project.stack) ?? undefined}
        >
          <span className="truncate">{project.stack.libraries.slice(0, 3).join(" · ")}</span>
          {project.stack.libraries.length > 3 && (
            <span className="shrink-0 text-muted/60">+{project.stack.libraries.length - 3}</span>
          )}
        </p>
      )}
```

The card root carries `select-none`, `hover:-translate-y-0.5`, an
`onPointerDown` that starts a drag, and `data-hangar-tile` attributes.
`menuOpen` + `menuRef` drive the `⋯` overflow menu from the header.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run build`, `npm run verify`, `npm run build:app`, `cargo`
anything, or `npm run test:acceptance` — a 600 s no-output watchdog has killed
executor runs here. Keep every Write/Edit under ~60 lines and commit after each.

## Scope

**In scope**:
- `src/components/ProjectCard.tsx` — the reveal
- `src/status.ts` — receives `relativeTime`, **moved** from `AddEditDialog.tsx`
- `src/components/AddEditDialog.tsx` — import swap only

**Out of scope** (do NOT touch):
- Anything under `src-tauri/`, `src/types.ts`, `src/store.ts`. No §7 command, no
  wire key, no state in the store — this is card-local UI over data already
  present in `project.stack`.
- The cap (stays 3), the badge, the port pill, the path line, the command line,
  the time slot, the phase strip, the footer. §11 fixes the element list.
- `src/cardDrag.ts`, `src/dragGeometry.ts`, `ProjectGrid.tsx`, `FolderTile.tsx`.
- Grouping the panel into "APIs" vs "Libraries". A TS mirror of ~27 Rust display
  names would drift silently and needs its own drift guard, for two names that
  are already visible in the top three. Explicitly cut.
- Any motion or transition on the panel. §11's Motion allow-list is exhaustive
  and the amendment says the panel "renders instantly with no transition".
- Any new dependency.

## Git workflow

- One commit per step: `Stack reveal: <what>`.

## Steps

### Step 1: Move `relativeTime` to the shared module

`relativeTime` lives in `AddEditDialog.tsx` (search for it — it renders
`detected 3 h ago`). Move it verbatim into `src/status.ts` beside
`lastRunLabel`, and import it back into `AddEditDialog.tsx`. **Do not copy it** —
two copies of a relative-time formatter drift.

**Verify**: `npm run typecheck` → exit 0; `git diff --stat` shows the function
leaving one file and entering the other.

### Step 2: Generalise the card's overlay state

`ProjectCard` currently has `menuOpen: boolean` and one `menuRef`. Replace with
a single `overlay: "menu" | "stack" | null` and add a **second ref** for the
libraries container.

**This second ref is mandatory, not tidiness.** The existing outside-click
effect tests `menuRef.current.contains(event.target)`, and `menuRef` sits on the
header div — it does **not** contain the libraries line. Reusing it unchanged
would treat every click *inside* the panel as an outside click and close it
instantly.

Update the `⋯` button and menu to read/write `overlay === "menu"`; the outside
handler must close whichever overlay is open, testing the matching ref.

**Verify**: `npm run typecheck` → exit 0.

### Step 3: The `+N` button and the panel

1. The libraries `<p>` becomes a `<div>` with `relative` added.
   **Both changes are load-bearing.** A `<ul>` inside a `<p>` is invalid nesting
   and the browser will hoist it out; and `w-full` on a panel whose containing
   block is an inline `<span>` resolves against the `+5` glyph (~20 px), not the
   card.
2. The `+N` `<span>` becomes `<button type="button">`. **Keep `shrink-0`** —
   that class is what stops the ellipsis eating `+N`.
3. The panel, rendered only when `overlay === "stack"`:

```
absolute left-0 bottom-full z-10 mb-2 w-full max-h-52 overflow-y-auto
select-text rounded-md border border-white/10 bg-bg p-2.5 shadow-lg
```

Each class earns its place — do not trim:

- **`bottom-full`, never `top-full`.** Downward the panel lands on the time
  slot, the `<footer>` holding Run/Stop, and the phase strip. §11 forbids
  occluding that button by name, and on a `stop-failed` card it is the only
  route out of the state.
- **`select-text`** — the card root carries `select-none`. Without this you
  cannot copy a single name out of a panel whose entire purpose is showing names.
- **`max-h-52 overflow-y-auto`** — an unbounded list would grow past the card.

4. Set `z-20` on the `<article>` **while the panel is open**. `hover:-translate-y-0.5`
   creates a stacking context, sealing a `z-10` child inside a `z-auto` card, so a
   later-DOM-order card paints over the panel.
5. **`onPointerDown={(e) => e.stopPropagation()}` on the panel root — mandatory.**
   `cardDrag.ts`'s guard bails only on `button, a, input, textarea, [role="menu"]`.
   A `<div><ul>` matches none of them, so a press-and-drift inside the panel
   starts a real drag and can file the project into a folder.
6. Esc: a scoped React `onKeyDown` that calls `stopPropagation()` **only when the
   panel is open** — copy the `onMenuKeyDown` shape already in this file. Closed,
   it must not stop propagation, or the folder band stops closing on Esc.
7. Focus the panel on open (`tabIndex={-1}`), return focus to the `+N` button on
   close.

**Contents**: the framework as the first chip when present, then one chip per
library, in the badge's exact tokens
(`rounded-full border border-white/10 bg-white/5 px-2 py-0.5 font-mono text-xs text-muted`),
inside a `<ul>`/`<li>`. Footer line: `detected 3 h ago` via `relativeTime`.

**Exact strings**: visible text stays `+5`. Button `aria-label`:
`+5 more — show the full stack for example-monorepo web`. Panel `aria-label`:
`Detected stack for example-monorepo web`.

**Verify**: `npm run typecheck` → exit 0.

### Step 4: Self-check

Report each:

- `grep -n "top-full" src/components/ProjectCard.tsx` → **no match**.
- `grep -n "select-text\|shrink-0\|bottom-full" src/components/ProjectCard.tsx` → all present.
- `grep -c "relativeTime" src/components/AddEditDialog.tsx src/status.ts` → defined once, in `status.ts`.
- `grep -n "stopPropagation" src/components/ProjectCard.tsx` → on both the panel's pointerdown and the scoped Esc.
- `grep -rnE "#[0-9A-Fa-f]{6}" src/components/ProjectCard.tsx` → none.
- `git status --short` → only the three in-scope files.

**Verify**: `npm run typecheck` → exit 0.

## Test plan

No JS test runner for component behaviour (SPEC.md §4). Manual checks for the
reviewer/maintainer:

- Click `+5` on an `example-monorepo` card → a panel opens **upward** listing
  Express, TypeScript, Zod, Playwright, Vitest plus the framework, with
  `detected … ago` beneath. The Run button stays visible and clickable.
- Click `+5` again → closes. Click outside → closes. Esc → closes, and does
  **not** also collapse an open folder band.
- Open the panel on a card **inside an open folder** → it is not painted over by
  a neighbouring card.
- Press and drag *starting inside the panel* → nothing is filed into a folder.
- Select a library name with the mouse and copy it → the text selects.
- Example App (2 libraries) shows no `+N` and no control at all.
- Open the `⋯` menu, then click `+N` → only one overlay is open at a time.

## Done criteria

- [ ] `npm run typecheck` exits 0
- [ ] The panel anchors upward and never covers Run/Stop
- [ ] `relativeTime` is defined exactly once
- [ ] The panel has its own ref; the `⋯` menu still opens, closes and Escs
- [ ] Nothing under `src-tauri/`, `src/store.ts` or `src/types.ts` modified
- [ ] `plans/README.md` status row for 037 updated

## STOP conditions

Stop and report back if:

- The panel needs a store field, a §7 command, or any Rust. It does not —
  `project.stack` is already on the card's props.
- Fitting it requires changing the grid track, the card padding, the cap, or
  removing any card element. §11 fixes all of those; report instead.
- You are tempted to group entries into "APIs" and "Libraries". Explicitly cut —
  see Scope.
- Reusing `menuRef` for the panel seems to work. It cannot; re-read step 2.

## Maintenance notes

- The `z-20`-while-open dance exists because `hover:-translate-y-0.5` makes the
  card a stacking context. The `⋯` menu has the same latent exposure today and
  was left alone deliberately — if it ever misbehaves at a card's bottom edge,
  this is why.
- If the reveal ever needs to show something not already in `stack`, that is a
  §11 re-amendment: the current text permits exactly "show what is already
  stored", and that bound is the whole reason the exception was granted.
