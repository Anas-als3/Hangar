# Plan 019: Swap the palette to the amended §11 values and make cards compact

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat cfe3a44..HEAD -- src/index.css src/components/ProjectGrid.tsx src/components/ProjectCard.tsx`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW mechanically; MED for legibility — a palette change can quietly
  wreck contrast, and a density change can quietly wreck readability. Both
  failure modes are invisible to every gate in this repo.
- **Depends on**: none (SPEC.md §11's palette was amended 2026-08-09 to the
  target values below; this plan implements that amendment)
- **Category**: direction (maintainer-requested)
- **Planned at**: commit `cfe3a44`, 2026-08-09

## Why this matters

The maintainer asked to change the application colours and make the cards
smaller. §11's palette was enumerated, so it was amended first (see the Palette
bullet in `SPEC.md` §11, which now carries the new values and a note that the
*structure* — dark base, one raised surface, one accent, functional status
colours — is what is load-bearing, not the specific hues).

Card density needed no amendment: the 2026-08-09 motion amendment already freed
"spacing, hierarchy, weight, borders, the exact composition within the card",
and "dense" is one of §11's three stated goals ("Dark, dense, calm").

This is a small change with a real trap: the whole codebase references colours
through tokens, so swapping them is easy — which makes it easy to ship
something illegible without noticing.

## Current state

`src/index.css` declares every colour in one `@theme` block. **This is the only
place colours are defined** — no component holds a hex value (verified by
`grep -rnE "#[0-9A-Fa-f]{6}" src/components/ src/App.tsx` → no matches, a rule
enforced at every review):

```css
@theme {
  --color-bg: #101623;
  --color-surface: #1a2233;
  --color-text: #e8ecf4;
  --color-muted: #8a94a8;

  /* Single accent: warm amber. Run button, active phase. */
  --color-accent: #f5b942;

  /* Status colors are functional only (§11). */
  --color-status-running: #34d399;
  --color-status-active: #f5b942; /* starting / updating / installing — amber pulse */
  --color-status-danger: #f87171; /* crashed & stop-failed */
  --color-status-stopped: #64748b; /* "stopped slate" — §11 gives no hex; slate-500 */

  --font-display: "Space Grotesk", system-ui, sans-serif;
  --font-sans: "Inter", system-ui, sans-serif;
  --font-mono: "JetBrains Mono", ui-monospace, monospace;
}
```

Card sizing today:
- `src/components/ProjectGrid.tsx:10` —
  `grid grid-cols-[repeat(auto-fill,minmax(20rem,1fr))] gap-4`
- `src/components/ProjectCard.tsx:163` — the `<article>` shell, currently
  `... flex flex-col gap-3 rounded-lg border border-white/5 bg-surface p-5 ...`
  (plus the `hangar-fade-in` and hover-lift classes added by plan 018 — keep them)

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |
| Full verify | `npm run verify` | exit 0 |

**Do not run** `npm run build`, `cargo test`, `cargo check`, or
`npm run test:acceptance` — a 600 s no-output watchdog has killed executor runs
on this repo. Keep every Write/Edit under ~60 lines, commit after each. Your
reviewer runs the full suite.

## Scope

**In scope**:
- `src/index.css` (the `@theme` colour tokens, and any comment that names the
  old palette)
- `src/components/ProjectGrid.tsx` (grid column sizing)
- `src/components/ProjectCard.tsx` (padding, internal spacing, secondary text
  sizes)

**Out of scope** (do NOT touch):
- Any Rust, anything under `src-tauri/`, `src/store.ts`.
- The three font tokens.
- **The functional status colours' meanings** — running stays green, crashed and
  stop-failed stay red. §11: "Status colors are functional only". Changing what
  a colour *means* is not a palette swap.
- Card *contents* or their order — still fixed by §11. Density is about spacing
  and type scale, not removing elements.
- Motion — plan 018 just landed it; do not add, remove or retime any transition.
- Any new dependency.

## Git workflow

- One commit per file: `Palette and density: <what>`.

## Steps

### Step 1: Swap the colour tokens

Replace the colour values in `@theme` with the amended §11 palette:

```css
  --color-bg: #0c0d11;      /* neutral graphite base */
  --color-surface: #16181e; /* one raised surface */
  --color-text: #e9eaee;
  --color-muted: #8a8f9c;

  /* Single accent: violet. Run button, active phase. */
  --color-accent: #8b7bf7;

  /* Status colors are functional only (§11) — they do NOT follow the accent. */
  --color-status-running: #34d399;
  --color-status-active: #8b7bf7; /* starting / updating / installing — pulses in the accent */
  --color-status-danger: #f87171; /* crashed & stop-failed */
  --color-status-stopped: #6b7280; /* "stopped slate" — §11 gives no hex */
```

Note `--color-status-active` tracks the accent by design (§11 describes the
transitional pulse as being in the accent colour), while running/danger do not.

Update any comment in the file that still says "amber" so the file does not
describe a palette it no longer has.

**Verify**: `npm run typecheck` → exit 0. Then
`grep -in "amber" src/index.css src/components/*.tsx src/App.tsx` → only
matches that are genuinely still accurate (there should be none referring to the
accent).

### Step 2: Compact the grid

In `src/components/ProjectGrid.tsx`, change the column track from
`minmax(20rem,1fr)` to `minmax(14rem,1fr)` and the gap from `gap-4` to `gap-3`.

Keep `auto-fill` and `1fr` — the grid must stay responsive, and cards must still
stretch to fill the row rather than leaving ragged gaps.

**Verify**: `npm run typecheck` → exit 0.

### Step 3: Compact the card

In `src/components/ProjectCard.tsx`:
- shell padding `p-5` → `p-3`
- shell `gap-3` → `gap-2`
- drop the secondary text one step where it does not hurt meaning: the path line
  and the command line in the footer may go to `text-[11px]` or stay `text-xs`
  — judgment, but they must remain legible, not decorative.
- keep the status pill readable; it is the thing users scan for. If the pill's
  plan-018 padding (`px-3 py-1.5`) now dominates a smaller card, reduce it to
  `px-2.5 py-1` — but do not shrink its text below `text-xs`.

**Do not** remove elements, change the header/footer structure, or alter the
hover-lift and fade-in classes.

At 14rem the path and command lines will truncate hard. That is expected and
accepted — both already use `truncate` with a `title` attribute for the full
value on hover. Confirm those `title` attributes are still present; if any
element truncates without one, add it.

**Verify**: `npm run typecheck` → exit 0.

### Step 4: Gates and commit

**Verify**: `npm run verify` → exit 0; `git status --short` shows only in-scope
files.

## Test plan

No automated test — there is no JS test runner (SPEC.md §4's dependency rule),
and no gate in this repo renders a component. That is exactly how a modal
shipped on 2026-08-09 that covered the whole app while every check passed.

Manual checks for the reviewer/maintainer (a subagent cannot see the GUI):
- Every surface is the new palette; nothing still renders amber.
- The Run button is clearly the primary action against the new base.
- A `running` card's green pill and a `crashed` card's red pill are both
  unmistakable against the new surface — these carry meaning and must not blend.
- Secondary text (path, command) is still readable, not merely present.
- At a normal window width the grid fits noticeably more cards per row.
- The phase strip still reads as the most expressive element (§11).

## Done criteria

- [ ] `npm run verify` exits 0
- [ ] `grep -rnE "#[0-9A-Fa-f]{6}" src/components/ src/App.tsx` → no matches
      (colours stay centralised in the `@theme` block)
- [ ] `grep -in "amber" src/` → no stale references to the old accent
- [ ] No Rust file, no `src/store.ts`, no font token modified
- [ ] Card element list and order unchanged
- [ ] `plans/README.md` status row for 019 updated

## STOP conditions

Stop and report back if:

- Making cards compact would require removing a card element — §11 fixes the
  list; report rather than dropping something.
- Any component turns out to hardcode a colour (the grep should find none) —
  that is a finding worth reporting, not silently fixing alongside this work.
- The status colours stop being distinguishable against the new surface. They
  are functional; if green/red do not read clearly, the base colour is wrong and
  the maintainer needs to know before this ships.

## Maintenance notes

- Because every colour is one `@theme` block and no component hardcodes hex, a
  future palette change is again a single-file edit. Preserve that: reject any
  future diff that introduces a hex literal into a component.
- If the compact size proves too tight in real use, the three numbers to revisit
  are the grid `minmax`, the card `p-*`, and the secondary text size — in that
  order. Density was chosen by the maintainer over a moderate alternative
  (17rem/p-4), so that is the known fallback.
