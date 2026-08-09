# Plan 018: Implement the motion SPEC.md §11 now permits, and a card visual pass

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat 23bb09b..HEAD -- src/index.css src/App.tsx src/components/`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: MED — not technically, but because this is the plan most able to
  damage the product's visual identity. §11's whole thesis is "Dark, dense,
  calm" with exactly one memorable element. Over-animating is the failure mode.
- **Depends on**: plans/016 (the §11 amendment — **ratified and applied to
  SPEC.md on 2026-08-09**; this plan is only in scope because of it)
- **Category**: direction (maintainer-requested)
- **Planned at**: commit `23bb09b`, 2026-08-09

## Why this matters

The maintainer asked for "ui smoothness like animations, and better card
design". SPEC.md §11 previously capped motion at "phase-strip fill + subtle card
hover lift only", so that work was a spec violation rather than an omission. §11
was amended deliberately (plan 016) to permit a short, explicit allowlist. This
plan implements exactly that allowlist and nothing beyond it.

Today every overlay pops in with no transition, status colours snap between
states, and cards appear and vanish instantly. None of that is broken — it is
just abrupt, and §11 now says motion may be used to explain a state change.

## The constraint that matters most

Read §11 in `SPEC.md` before writing any CSS. Two sentences govern this plan:

> This is the one memorable element; it encodes the actual sequence, not
> decoration. Keep everything else quiet.

and, from the amended Motion bullet:

> the phase-strip fill (the signature element — it stays the most expressive
> motion in the app, and nothing else may compete with it)

**If a reviewer opens the app and the dialog transitions draw more attention
than the phase strip, this plan has failed** even if every gate passes. Aim for
motion the user does not consciously notice — it should remove abruptness, not
add personality.

The amended §11 also requires **CSS transitions, not JS animation**. This is a
performance requirement: the 2026-08-06 audit's PERF-01 (still unplanned) found
that `src/store.ts`'s `setState` notifies every subscriber on every patch, so a
`log-lines` flush re-renders the whole grid up to ~10× per second per running
project. CSS transitions are unaffected by re-render; JS-driven animation is not.

## Current state

- `src/index.css` holds the tokens, a global `prefers-reduced-motion` rule
  (~line 49) that kills all animation and transition durations, plus
  `hangar-pulse` (~line 72, transitional statuses) and `hangar-spin` (~line 84,
  the `stopping` button). Those two loops are explicitly still allowed by §11 —
  do not remove them.
- `src/components/ProjectCard.tsx` line ~163 is the card shell, and already has
  the §11 hover lift:

```tsx
<article className="relative flex flex-col gap-4 rounded-lg border border-white/5 bg-surface p-5 transition-transform duration-150 hover:-translate-y-0.5">
```

- Three overlay surfaces, all currently rendering with no enter transition:
  - `src/components/LogPanel.tsx:94` — `fixed inset-0 z-20 flex justify-end` (the §11 slide-over)
  - `src/components/AddEditDialog.tsx:125` — `fixed inset-0 z-20 flex items-center justify-center`
  - `src/components/SettingsDialog.tsx:49` — same shape
  All three return `null` when closed (`AddEditDialog` at ~line 72, `SettingsDialog`
  at ~line 40 — the latter added by the blocker fix on 2026-08-09).
- `src/App.tsx:172` renders `{toast && <Toast message={toast} />}`.
- `src/components/ProjectCard.tsx` has `STATUS_TONE`, a `Record<Status, string>`
  of text-colour classes, applied to the status pill.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |
| Full verify | `npm run verify` | exit 0 |

**Do not run** `npm run build`, `cargo test`, `cargo check`, or
`npm run test:acceptance` — a 600 s no-output watchdog has killed six executor
runs on this repo. Keep every Write/Edit under ~60 lines and commit after each.
Your reviewer runs the full suite.

## Scope

**In scope**:
- `src/index.css` (keyframes/utility classes for the allowed transitions)
- `src/components/LogPanel.tsx`, `AddEditDialog.tsx`, `SettingsDialog.tsx`
  (enter transitions on the overlay + backdrop)
- `src/components/ProjectCard.tsx` (status-colour transition; visual pass)
- `src/components/PhaseStrip.tsx` (segment colour transition only)
- `src/App.tsx` (toast enter; card enter/exit wrapper if needed)

**Out of scope** (do NOT touch):
- Any Rust, any file under `src-tauri/`.
- `src/store.ts` — no state changes are needed for CSS transitions.
- The §11 palette tokens, the three fonts, the status colour meanings.
- **Card contents or their order** — §11 still fixes the element list (name,
  status pill with port, time slot, primary button, overflow menu) and forbids
  automatic re-sorting. Only *visual treatment* was freed by the amendment.
- The phase strip's own fill behaviour and its lit/dimmed/pending semantics.
- Any new dependency. No animation library, no `framer-motion`, nothing.
- Exit animations that require keeping unmounted components alive — see step 2.

## Git workflow

- One commit per file: `UI polish: <what>`.

## Steps

### Step 1: Transition utilities in `index.css`

Add a small number of utility classes for the allowed motion. Keep durations
≤200 ms and use ease-out. Suggested shape (adapt names to the file's style):

- `.hangar-fade-in` — opacity 0 → 1, ~150 ms.
- `.hangar-dialog-in` — opacity 0 → 1 plus `translateY(4px)` → 0, ~160 ms.
- `.hangar-slide-in` — `translateX(100%)` → 0, ~180 ms, for the §11 slide-over.

Do **not** add a global `* { transition: all }` — it would animate things §11
does not permit and would fight the reduced-motion rule.

Confirm the existing `prefers-reduced-motion` block already neutralises these
(it sets `animation-duration` and `transition-duration` to `0.01ms !important`
for `*`). If it does, add nothing further; if any new class escapes it, extend
the block rather than special-casing.

**Verify**: `npm run typecheck` → exit 0.

### Step 2: Overlay enter transitions

Apply the utilities to the three overlay surfaces: backdrop fades, panel enters.
`LogPanel` slides from the right (§11 calls it a slide-over); the two dialogs
use the dialog-in utility.

**Enter only.** All three components `return null` when closed, so an exit
transition would require keeping them mounted and tracking an "exiting" state.
That is state machinery for a 150 ms effect, it risks resurrecting the
always-rendered bug fixed on 2026-08-09, and §11 does not require exits. If you
find yourself adding a `useState` to animate a disappearance, STOP — that is out
of scope.

**Verify**: `npm run typecheck` → exit 0.

### Step 3: Status colour transitions

Add a colour/opacity transition (≤200 ms) to the status pill in `ProjectCard`
and to the phase segments in `PhaseStrip`, so a status change fades rather than
snaps. `transition-colors duration-200` on the relevant elements is sufficient.

Do not change any colour *value* or which status maps to which token — only how
the change is applied over time.

**Verify**: `npm run typecheck` → exit 0.

### Step 4: Card enter/exit

Cards should fade in when a project is added. Use CSS only — a `.hangar-fade-in`
on the card shell is enough, since a newly added card mounts fresh.

True exit animation on Remove would require keeping the removed card mounted;
per step 2's reasoning, skip it. Removal staying instant is acceptable and
honest — the project is gone.

**Verify**: `npm run typecheck` → exit 0.

### Step 5: The card visual pass

This is the subjective part; the amendment freed "spacing, hierarchy, weight,
borders, the exact composition within the card" while fixing the element list.

Improve *hierarchy*, not decoration. Concretely, the current card gives the
project name, path, status pill, time slot, command and buttons fairly uniform
visual weight. Suggested direction (use judgment, keep it quiet):

- Make the project name clearly primary; de-emphasise the path (it is reference
  information, currently `font-mono text-xs text-muted` — that is close already).
- Give the status pill and its port more presence, since that is the state the
  user is scanning for across a grid.
- Tighten vertical rhythm so cards read faster in a dense grid.
- Keep the primary Run/Stop button unmistakably the primary action.

Hard limits: no gradients, no glassmorphism, no shadows beyond what the tokens
already imply, no new colours, no new elements, no icons that were not there.
Every colour must come from the existing tokens — **no raw hex** (a reviewer
greps for `#[0-9A-Fa-f]{6}`).

**Verify**: `npm run typecheck` → exit 0.

### Step 6: Gates and commit

**Verify**: `npm run verify` → exit 0; `git status --short` shows only in-scope
files.

## Test plan

No automated test — there is no JS test runner (SPEC.md §4's dependency rule;
see the amendment in `plans/README.md`'s rejected-findings section). This is also
why the 2026-08-09 `SettingsDialog` blocker shipped: `tsc`, 93 Rust tests, a
production build and three CI jobs all passed while a modal covered the app.
**Assume nothing about rendering that you have not reasoned through carefully.**

Manual checks for the reviewer/maintainer (a subagent cannot drive the GUI):
- Open and close each dialog and the log slide-over: they ease in, and closing
  is immediate with no stuck overlay.
- Run a project: the status pill fades between colours; the phase strip is still
  the most eye-catching motion on screen.
- Add a project: its card fades in. Remove one: it disappears immediately.
- Enable Reduce Motion in macOS Accessibility settings: **all** of the above
  becomes instant, and the app remains fully usable.
- Cards still show exactly the §11 elements, in `projects.json` order.

## Done criteria

- [ ] `npm run verify` exits 0
- [ ] `grep -rnE "#[0-9A-Fa-f]{6}" src/components/ src/App.tsx` → no matches
- [ ] `grep -rn "framer-motion\|animejs\|gsap" package.json` → no matches
- [ ] No `useState` was added to animate a component's disappearance
- [ ] No file under `src-tauri/` modified; `src/store.ts` unmodified
- [ ] Every transition duration is ≤200 ms
- [ ] `plans/README.md` status row for 018 updated

## STOP conditions

Stop and report back if:

- You need to keep a closed dialog mounted to animate it out (step 2).
- You want motion not on §11's allowlist — the allowlist is exhaustive; amending
  §11 again is a maintainer decision, not an executor one.
- The card pass starts requiring new elements, icons or colours — §11 still
  fixes the element list.
- `prefers-reduced-motion` does not neutralise something you added and extending
  the global rule does not fix it.

## Maintenance notes

- The riskiest regression here is subjective and no gate can catch it: motion
  creeping past "explains a state change" into "has personality". A reviewer
  should open the app and check the phase strip still dominates.
- If card motion ever looks janky with a chatty dev server running, the cause is
  audit finding PERF-01 (store fan-out re-rendering every card per log flush),
  not the CSS. That finding is still unplanned.
- Exit transitions were deliberately skipped. If they are ever wanted, they need
  a considered approach to keeping components mounted — and a re-read of the
  2026-08-09 blocker, where an always-rendered dialog made the app unusable.
