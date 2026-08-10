# Plan 046: Presence — stop the app reading as dead

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "every card carries" SPEC.md && grep -n "minmax(14rem,1fr)" src/components/ProjectGrid.tsx`
> The §11 amendment must be present and the track must appear **twice**. On a
> mismatch, STOP.
>
> **Gate ownership**: you run `npm run typecheck`. **Your reviewer runs
> `npm run build`, `cargo check` and the bundle.**

## Status

- **Priority**: P2 — the maintainer's own words: *"the application is like a
  dead application… everything is black"*
- **Effort**: M
- **Risk**: MED — two of these are live bugs, and two have hard couplings that
  fail silently
- **Depends on**: SPEC.md §11 amendments (ratified 2026-08-10)
- **Category**: bug + direction
- **Planned at**: 2026-08-10, after a nine-agent design pass and a maintainer
  ruling

## Why this matters

At three stopped projects on the default 1200×800 window, **59 % of the window
is black** — the grid draws one 200 px row into a 673 px box. The maintainer
ruled that this space is a **margin, not a bug**: the fix is presence, not
filling it. Everything below adds signal at the existing scale and palette.

Two of the items are outright bugs, found while measuring:

- **Pressing Run grows every other card in its row by 40 px.** Grid rows
  stretch; a stopped card is 200 px and a running one 240 px. Starting one
  project pushes the Run buttons down on cards nothing happened to.
- **The overflow menu will paint underneath the next row.** `MENU_ITEMS` is
  seven items ≈ 234 px of menu on a 200 px card, so it overhangs ~83 px. The
  card's `hover:-translate-y-0.5` makes it its own stacking context, and
  `ProjectCard.tsx` lifts the card to `z-20` only when the **stack panel** is
  open — not the menu. Latent at one row; live the moment there are two.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run build`, `npm run build:app`, `npm run install:app`,
`cargo` anything, `npm run verify`, or `npm run test:acceptance`. Keep every
Write/Edit under ~60 lines and commit after each.

## Scope

**In scope**:
- `src-tauri/tauri.conf.json` — two window keys
- `src/App.tsx` — title size, the width cap
- `src/components/ProjectCard.tsx` — padding, name size, the menu z-index
- `src/components/PhaseStrip.tsx` — always render, unlit at rest
- `src/components/ProjectGrid.tsx` — only if the track must change (it must not)

**Out of scope** (do NOT touch):
- **The grid track.** `minmax(14rem,1fr)` stays. At exactly three projects,
  4 columns is the unique perfect fit — 3 cards + the Add tile = one complete
  row. An 18 rem floor gives 3 columns and strands the Add tile alone on row 2,
  where its own `min-h-[8rem]` sets that row's height — **worse**, and
  permanently ragged. 17 rem changes nothing at 1200 px and silently drops the
  900 px window to 2 columns.
- Any new colour, gradient, glassmorphism, background texture, or second
  accent. §11 still bans them and the maintainer's ruling was *presence*, not
  decoration.
- Any motion. The strip at rest is **static**.
- `grid-auto-flow: dense`, vertical centring of the grid, cards that stretch to
  fill the row, density derived from item count. All four were considered and
  rejected in the design pass; re-proposing them is out of scope.
- The status pill, port button, stack badge, libraries line, folder tiles, the
  drag subsystem, the Ports panel, `window.confirm` (a later plan).
- Any new dependency. Any Rust beyond the two config keys.

## Git workflow

- One commit per step: `Presence: <what>`.

## Steps

### Step 1: The menu z-index bug

In `ProjectCard.tsx`, the `<article>` lifts to `z-20` only while the stack
panel is open. Extend it to lift while **either** overlay is open — the menu
included.

**Verify**: `npm run typecheck` → 0; `grep -n "z-20" src/components/ProjectCard.tsx`
shows the condition covering both.

### Step 2: The window paints dark from the first frame

In `src-tauri/tauri.conf.json`'s window config add:

```json
"backgroundColor": "#0C0D11",
"theme": "Dark"
```

Without the first, the WKWebView default (white) paints before `index.css`
parses — a white flash on every launch. Without the second, on macOS Light the
**native title bar is light grey** bolted to a near-black app.

This is not a §11 change: it makes the *existing* palette apply where it
currently does not. Add a comment noting the hex is paired with `index.css`'s
`--color-bg` and must change with it. (Plan 019 bans raw hex in **components**;
a config file is not a component.)

**Verify**: the JSON parses — `python3 -c "import json;json.load(open('src-tauri/tauri.conf.json'))"`.

### Step 3: Give the page a top

In `App.tsx`, the `<h1>` goes from `text-xl` to `text-2xl` (20 → 24 px).

Today the **largest type in the entire application is the `+` glyph on the Add
tile**, and the app title is 2 px larger than a project name. That is why the
page reads as having no top.

**The header height must not change.** `text-2xl`'s line box is 32 px, still
under the 38 px search input, so the header stays 79 px. **Keep the running
count inline** — moving it below the title would grow the header by 10 px
whenever a project starts.

**Do not restructure the header out of its wrapper.** That div receives `inert`
while an overlay is open (plan 039).

**Verify**: `npm run typecheck` → 0.

### Step 4: Cap and centre the content

There is **no `max-width` anywhere in the tree**. At 2560 px the grid packs
**10 columns** and 6 sit empty; the horizontal void grows with the monitor.

Apply `max-w-[80rem] mx-auto` to **one shared wrapper used by both the header
row and `<main>`'s content**.

**Capping only the grid is the trap**: it would move every Run button inward
while "Add project" stays pinned to the far right edge.

Arithmetic to confirm in your report:

| width | today | after |
|---|---|---|
| 1200 | 4 cols | **unchanged** (1136 < 1280) |
| 1920 | 7 cols | 5 cols |
| 2560 | 10 cols, 6 empty | 5 cols, 1 empty |

The track floor stays 224 px in every case, so §11's "14 rem card" sentence
stays true.

**Verify**: `npm run typecheck` → 0.

### Step 5: The phase strip is always present

Per the §11 amendment: `PhaseStrip` renders on **every** card, entirely unlit
while `stopped`.

Two things that will otherwise ship broken:

1. **Force-unlit at `stopped`.** `phasesSeen` is cleared only on the transition
   *out of* `stopped`/`crashed`, so running → stopping → stopped leaves all four
   phases in the array. Rendering naively would show **four accent-lit segments
   reading "Ready" under a slate "Stopped" pill** on every project run this
   session. When `status === "stopped"`, treat `seen` as empty.
2. **Do not change `VISIBLE`'s meaning for the other statuses.** Only the
   `stopped` case is new.

This is what makes the card silhouette fixed and kills the 40 px jump.

**Verify**: `npm run typecheck` → 0; `grep -n "stopped" src/components/PhaseStrip.tsx`
shows the force-unlit branch.

### Step 6: Re-compose the card

`p-3` → `p-4`, `gap-2` → `gap-2.5`, project name `text-lg` → `text-xl`.

The name bump is **free height**: Tailwind gives `text-lg` and `text-xl` the
same 28 px line box.

**Two hard couplings, both silent failures if missed:**

1. **`PhaseStrip.tsx` hardcodes `-mx-3 -mb-3 … px-3`** with a comment saying
   these must equal `ProjectCard`'s `p-3`. Change both together or the
   signature element stops bleeding to the card's edge.
2. **`ProjectGrid.tsx` hardcodes `minmax(14rem,1fr)` a second time** for the
   folder band. You are not changing the track — but confirm both sites still
   match, because §11 requires a card be identical inside a folder and outside
   it.

**Verify**: `npm run typecheck` → 0; report both coupling sites and that they
agree.

### Step 7: Self-check

Report each:

- `grep -rn "gradient\|backdrop-blur\|glass" src/` → **no matches**.
- `grep -c "minmax(14rem,1fr)" src/components/ProjectGrid.tsx` → `2`, and the
  two agree.
- `grep -n "p-4\|-mx-4\|px-4" src/components/ProjectCard.tsx src/components/PhaseStrip.tsx`
  → the padding and the strip's negative margins agree.
- `grep -n "max-w-" src/App.tsx` → one wrapper, used by header and main.
- `git status --short` → only in-scope files.

**Verify**: `npm run typecheck` → 0.

## Test plan

No JS test runner for component behaviour (SPEC.md §4). Manual checks for the
reviewer/maintainer — the first two are the bugs:

- Press **Run** on one project with three visible. **No other card moves.**
  Before this, they all grew 40 px.
- With enough projects for two rows, open a card's `⋯` on the top row. The menu
  paints **over** the row below, not under it.
- Launch the app cold → **no white flash**, and the title bar is dark even on
  macOS Light.
- A `stopped` card shows an unlit four-segment strip. It must **not** show
  lit segments or the word Ready under a "Stopped" pill — including for a
  project you ran and stopped earlier in the same session.
- Maximise on the widest monitor available → content stays centred at ~1280 px
  rather than sprawling to ten columns.
- At 900×600 → still 3 columns, nothing clipped.

## Done criteria

- [ ] `npm run typecheck` exits 0
- [ ] Pressing Run moves no other card
- [ ] The card menu is not painted over by a later card
- [ ] No white launch flash; dark title bar
- [ ] A stopped card's strip is fully unlit, including after a run this session
- [ ] The grid track is unchanged and its two sites agree
- [ ] No gradient, blur, glass, new colour, or motion added
- [ ] `plans/README.md` status row for 046 updated

## STOP conditions

Stop and report back if:

- You are tempted to widen the grid track, centre the grid vertically, or make
  cards stretch. All three were measured and rejected — see Scope.
- The `p-3` → `p-4` change cannot be matched in `PhaseStrip.tsx`. Report; a
  mismatched strip is more visible than the padding gain.
- Making the strip always-present seems to need a store change. It does not —
  the force-unlit is a render-time condition on `status`.
- Any of this seems to need a new colour or a background treatment. The
  maintainer ruled for **presence**, not decoration, and §11 still bans both.

## Maintenance notes

- **The card's padding and `PhaseStrip`'s negative margins are one number in
  two files.** There is already a comment saying so; this plan is the first
  time it has actually been exercised.
- The width cap is the only fix that scales with the monitor. Everything else
  is fixed-size and self-corrects as the library grows.
- What this deliberately does **not** fix: the window is still ~52 % black at
  three projects, and geometry cannot close that — four tiles cannot fill
  673 px at any defensible card size. What changes is that the black sits
  *around* something instead of *instead of* it.
