# Plan 030: Drag a card onto another card to make a folder

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "gridItems\|moveToFolder" src/store.ts`
> Both must exist. If they do not, plan 029 has not merged — **STOP**.

## Status

- **Priority**: P2 — the gesture is the thing the maintainer actually asked
  for; plan 029 shipped folders, not the gesture
- **Effort**: M
- **Risk**: MED-HIGH — a webview drag subsystem no gate in this repo can
  exercise, on a platform whose drag interception is source-traced but not
  runtime-verified
- **Depends on**: plans 028 + 029 (both DONE)
- **Category**: feature
- **Planned at**: commit `f4b33e5`, 2026-08-10

## Why this matters

The maintainer's words: *"add a folder option that when i drag a card to
another card they get merged in a folder together just like how ios does with
iphone when i drag an app to another app they get grouped in a folder."*

Plan 029 delivered folders through the overflow menu. This plan delivers the
gesture. **Read SPEC.md §11's Motion bullet on drag-to-group feedback** — it was
amended on 2026-08-10 for this plan and it constrains every visual decision
here.

## Why pointer events and not HTML5 drag-and-drop

Traced through the vendored crates: `src-tauri/tauri.conf.json` sets no
`dragDropEnabled`, so it defaults to `true`; `tauri-runtime-wry` installs a
drag-drop handler that returns `true` unconditionally; `wry`'s WKWebView
`drag_drop` module only forwards to WebKit's own `NSDraggingDestination` — the
thing that turns an AppKit drag session into DOM `dragenter`/`dragover`/`drop`
— when that handler returns `false`.

**The unverified part** is whether WebKit routes *intra-page* drags through that
destination at all on macOS. Nobody has run it.

Pointer events are the right choice either way, and that is why this plan does
not wait on the probe: identical code path on macOS and Windows (nothing on this
machine can execute the Windows path at all), no window-level runtime flag
flipped just to make a card grid work, and deterministic geometry and timing
that a unit test can actually reach.

## Current state

`src/components/ProjectGrid.tsx` renders `ProjectCard` and `FolderTile` from a
`gridItems(...)` walk. Neither tile carries any drag affordance or `data-`
attribute today.

`src/store.ts` exports `moveToFolder(projectId, target)` with

```ts
export type FolderTarget =
  | { kind: "existing"; folderId: string; folderName: string }
  | { kind: "new"; name: string }
  | { kind: "none" };
```

and reads the project fresh via `findProject` at call time. **That function is
the entire write path for this plan.** Do not add a second one.

`src/index.css` — the `prefers-reduced-motion` block clamps
`animation-duration: 0.01ms` on `*`.

`grep -rn "select-none" src/` finds nothing today, so WebKit will happily start
its own text selection drag unless a tile opts out.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |
| Bundle | `npm run build` | exit 0 |
| Geometry unit tests | `node --test src/dragGeometry.test.mjs` | all pass |

**Do not run** `npm run verify`, `npm run build:app`, `cargo` anything, or
`npm run test:acceptance` — a 600 s no-output watchdog has killed executor runs
here. This plan touches no Rust. Keep every Write/Edit under ~60 lines and
commit after each.

`node --test` is Node v24's **built-in** runner. It adds no dependency, which is
why SPEC.md §4's rule permits it — see `plans/README.md`'s recorded amendment on
exactly this point.

## Scope

**In scope**:
- `src/dragGeometry.ts` — new, pure, **zero imports**
- `src/dragGeometry.test.mjs` — new
- `src/cardDrag.ts` — new, the pointer session
- `src/store.ts` — a small `drag` view-state slice only
- `src/components/ProjectCard.tsx` — a `data-` attribute and a pointer-down hook
- `src/components/FolderTile.tsx` — the same, as a drop target only
- `src/components/ProjectGrid.tsx` — wiring if needed
- `src/index.css` — the ghost's class and the armed ring, if tokens are missing

**Out of scope** (do NOT touch):
- Anything under `src-tauri/`, and `src/types.ts`. The persisted shape is
  finished.
- `moveToFolder` / `renameFolder` / `ungroupFolder` / `gridItems` /
  `folderSummary` — reuse; do not rewrite.
- `MoveToFolderDialog.tsx`. The menu route must keep working untouched — it is
  the accessible equivalent of this gesture and the only route for a
  non-mouse user.
- Drag-to-**reorder**. SPEC.md §16 parks it. This plan reserves the
  between-tiles case in the type and returns it **never**.
- Edge auto-scroll. Explicitly deferred — see "Honest limits" below.
- Any new dependency. No drag library.

## Git workflow

- One commit per step: `Card drag: <what>`.

## Steps

### Step 1: The pure geometry module

New `src/dragGeometry.ts` with **no imports at all** — that is a hard
requirement, not a style note. `src/store.ts` imports `./api`, which imports
`@tauri-apps/api`, which `node --test` cannot resolve under
`moduleResolution: "bundler"`. A leaf module is the only part of this feature a
machine can verify.

Export:

```ts
export const DWELL_MS = 450;
export const DWELL_SLOP_PX = 6;
export const DRAG_THRESHOLD_PX = 5;

export type DropIntent =
  | { kind: "none" }
  | { kind: "merge"; targetKind: "project" | "folder"; targetId: string }
  | { kind: "insert"; beforeId: string | null };
```

Plus two pure functions:

- `hitTest(point, tiles)` → `DropIntent`, where `tiles` is a plain array of
  `{id, kind, rect}`. v1 returns `merge` when the point is inside a tile that is
  not the drag source, and `none` otherwise. **It must never return `insert`** —
  the variant exists so §16's parked reorder can land later without making
  today's folder drags ambiguous, and a comment must say exactly that.
- `hasMovedEnough(origin, point)` → boolean, using `DRAG_THRESHOLD_PX`.
- `stillWithinSlop(anchor, point)` → boolean, using `DWELL_SLOP_PX`.

`targetKind` is **not** optional. Without it every folder is capped at exactly
two projects forever, because dropping onto an existing folder becomes
unrepresentable.

**Verify**: `npm run typecheck` → exit 0.

### Step 2: Tests for the geometry

New `src/dragGeometry.test.mjs` using `node:test` and `node:assert`. Cover at
minimum:

1. A point inside a tile that is not the source → `merge` with that tile's id
   and kind.
2. A point inside the **source's own** tile → `none`. (Dropping a card on itself
   must not create a one-member folder.)
3. A point in the gutter between tiles → `none`, **not** `insert`.
4. A point over a folder tile → `merge` with `targetKind: "folder"`.
5. `hasMovedEnough` is false at 4 px and true at 6 px.
6. `stillWithinSlop` is true at 5 px and false at 7 px.

Import the module by its compiled-free path — since `dragGeometry.ts` is
TypeScript, either add a matching `.mjs` shim or write the test against the
built output. **If neither works cleanly, STOP and report** rather than adding a
transpiler dependency; the fallback is to write `dragGeometry` as plain `.mjs`
with a `.d.ts`, which is ugly but dependency-free.

**Verify**: `node --test src/dragGeometry.test.mjs` → all pass.

### Step 3: The pointer session

New `src/cardDrag.ts`. A module-level session object, **not** React state:

- Coordinates never enter the store. `setState` notifies every subscriber and
  log flushes already re-render the whole grid; a per-pointermove `setState`
  would re-render every card dozens of times a second.
- Only `{sourceId, targetId, armed}` reaches the store, which changes a handful
  of times per drag.

Rules, each of which exists for a reason — do not drop any:

- `pointerdown` on a card starts a *candidate*, not a drag. Bail immediately on
  `e.button !== 0`, on `e.ctrlKey` (macOS right-click), and on
  `e.target.closest('button, a, input, textarea, [role="menu"]')` — otherwise
  Run, Stop, the port link and the `⋯` menu all become drag handles.
- Listeners go on `window`, **not** `setPointerCapture`. Capture binds the event
  stream to an element React may unmount mid-drag (a crash, a status change, a
  registry reload).
- The ghost appears only after `hasMovedEnough`. It is **one detached node,
  written imperatively, outside React** — compact (~180 px: name plus a status
  dot), positioned at cursor + (10, 10), one style write per pointer event, no
  `requestAnimationFrame` loop. A full 14 rem card clone would cover the drop
  target, and the ring on that target is the thing the user needs to see.
- Hit-test with `document.elementFromPoint` per throttled move, over elements
  carrying a `data-hangar-tile` attribute. **Never cache rects**: a project can
  crash and vanish mid-drag under an active search.
- Dwell: the target arms after `DWELL_MS` stationary within `DWELL_SLOP_PX`.
  Moving off the target or beyond the slop cancels and restarts the timer.
- **Reduced motion: keep the 450 ms timer, drop only the animated ring.**
  Setting `DWELL_MS = 0` would arm the merge the instant the pointer entered a
  tile, for exactly the users least able to make a fine correction, in a feature
  whose undo is weak. `index.css` clamps animations to 0.01 ms, so an animated
  ring would read "armed" 450 ms before the drop actually arms — the visual must
  lag the timer, never lead it.
- Commit only when armed. Reset on `pointerup` without arming, `pointercancel`,
  window `blur`, `Escape`, and any `projects` change that removes **either** the
  source or the target.
- Suppress the trailing synthetic `click` after a drag, or dropping a card will
  also fire whatever it landed on.

**Verify**: `npm run typecheck` → exit 0.

### Step 4: Wire the tiles

- `ProjectCard`'s root `<article>` gains `data-hangar-tile` with its id and
  `kind="project"`, an `onPointerDown`, and `select-none` plus
  `[-webkit-user-drag:none]`. There is no `select-none` anywhere in `src/`
  today, so without it WebKit starts its own text-selection drag over the
  project name.
- `FolderTile`'s root gains the same `data-hangar-tile` with `kind="folder"` and
  the same user-select opt-out, but **no `onPointerDown`** — a folder is a drop
  target in v1, not a drag source. Dragging folders around is reordering, which
  §16 parks.
- Feedback, per §11's amended Motion bullet: the dragged card drops to ~40 %
  opacity and an armed target shows a 2 px accent ring, **applied instantly —
  no transition, opacity and colour only**, no scale, no lift, no spring.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 5: The write path

On an armed drop, call the **existing** `moveToFolder`:

- Source project onto a **folder tile** → one call, `{kind:"existing", ...}`.
- Source project onto a **plain card** → the target project first, then the
  source, both into a new folder id — two sequential calls with
  `{kind:"new", name}` for the first and `{kind:"existing", ...}` for the
  second, so both land in the *same* folder.
- Default new-folder name: `"New Folder"`. Do not prompt mid-gesture; §11 puts
  rename on the tile, and a modal in the middle of a drag is worse than a
  boring default.
- A partial failure (the second call fails) leaves a **one-member folder**,
  which is a valid state under §5's model. Toast and move on; there is nothing
  to repair.
- Announce the result with the **neutral** toast tone plan 029 added. The
  announcement is the accessibility story for a mouse-only gesture — do not skip
  it, and do not use the error tone.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 6: Gates and self-check

Report each:

- `node --test src/dragGeometry.test.mjs` → all pass
- `grep -n "^import" src/dragGeometry.ts` → **no output** (zero imports)
- `grep -rn "setPointerCapture\|requestAnimationFrame" src/` → no matches
- `grep -rn "insert" src/cardDrag.ts` → the intent is never constructed
- `grep -rn "grid-auto-flow\|\.sort(" src/` → still no code usage
- `git status --short` → only in-scope files

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

## Test plan

Step 2's unit tests are the only machine-checkable part, and that is exactly why
the geometry was split into a leaf module. Everything else needs hands.

Manual checks for the maintainer:

- Drag one card onto another, hold ~half a second. The target rings; release
  merges both into "New Folder" at the **first** one's grid position.
- Drag a third card onto that folder tile. It joins; the folder does not become
  a second folder.
- Drag a card onto **itself** → nothing happens.
- Release in the gutter → nothing happens.
- Drag across the grid without pausing → nothing arms.
- Press Esc mid-drag → the ghost disappears, nothing is written.
- Start a drag on the Run button, the `⋯`, or the `:3000` port link → those
  still work as buttons, no drag starts.
- Drag while a project is running → it still merges (folder fields are run-inert
  per §6) and the run is unaffected.
- With Reduce Motion on in macOS System Settings, the dwell still takes the same
  ~450 ms — it must not arm instantly.

## Done criteria

- [ ] `npm run typecheck` exits 0
- [ ] `npm run build` exits 0
- [ ] `node --test src/dragGeometry.test.mjs` passes, ≥6 cases
- [ ] `dragGeometry.ts` has zero imports
- [ ] No `setPointerCapture`, no rAF loop, no drag library
- [ ] `MoveToFolderDialog` and the menu route are byte-identical
- [ ] Nothing under `src-tauri/` or in `src/types.ts` modified
- [ ] `plans/README.md` status row for 030 updated

## STOP conditions

Stop and report back if:

- The geometry module cannot be reached by `node --test` without a transpiler.
  Report the constraint; do not add a dependency to make a test run.
- Making the gesture work seems to need `dragDropEnabled: false` in
  `tauri.conf.json`, or any Tauri config change. Pointer events need none, and
  flipping a window-level flag to make a card grid work is a much larger
  decision than this plan.
- You find yourself writing pointer coordinates into the store on every move.
  Re-read step 3 — that is the one performance rule here.
- The drag needs to reorder the `projects` array. It must not: §11 forbids
  re-sorting and §16 parks reordering.

## Honest limits, to be recorded rather than hidden

- **No edge auto-scroll in v1.** At the 1200×800 default the grid holds roughly
  12 tiles and at the 900×600 minimum roughly 6, so an off-screen drop target is
  a real case, not an edge case. That is precisely why "Move to folder…" is a
  required route and not a nicety.
- **Discoverability is weak.** `cursor: grab` and a menu item are the only hints.
  iOS gets away with no affordance because everyone already knows the gesture.
- **`pointercancel` under macOS system gestures** (Force Touch, three-finger
  drag, Mission Control) is unverified and unverifiable from here. The
  blur/Esc/cancel resets are the mitigation; expect one round of real-hardware
  tuning.
- **This consumes the drag gesture** §16 reserved for reordering. The `insert`
  variant keeps the door open, but retrofitting the discrimination later makes
  every existing folder drag momentarily ambiguous.

## Maintenance notes

- `dragGeometry.ts` staying import-free is what keeps this feature testable. If
  a future change gives it an import, the tests stop running and nobody will
  notice until something breaks by hand.
- If §16's drag-to-reorder is ever promoted, `hitTest` is the single place that
  changes: drop on a tile's centre stays `merge`, drop in the gutter starts
  returning `insert`.
- The gesture and the menu must stay behaviourally identical — both go through
  `moveToFolder`. If they ever diverge, the menu is the correct one, because it
  is the only route a non-mouse user has.
