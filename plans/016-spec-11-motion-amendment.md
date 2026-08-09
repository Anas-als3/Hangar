# Plan 016: Amend SPEC.md §11's motion clause — a proposal for the maintainer to ratify

> **This plan is a SPEC AMENDMENT PROPOSAL, not an implementation task.** Nothing
> in it should be executed by a coding agent. The maintainer reads it, edits the
> wording if they disagree, and applies §Section A to `SPEC.md`. Only after that
> is the follow-on UI work in-scope; until then §11 as written forbids it.

## Status

- **Priority**: P2
- **Effort**: S (the edit); the work it unblocks is separate
- **Risk**: MED — this is the first amendment to a spec that has been treated as
  authoritative through six milestones. The risk is not technical; it is that
  loosening §11 by habit turns "UI must follow §11 exactly" into a phrase nobody
  checks.
- **Depends on**: none
- **Category**: docs
- **Planned at**: commit `fe734e4`, 2026-08-09
- **Requested by**: the maintainer, 2026-08-09 ("add some ui smoothness like
  animations, and better card design")

## Why this needs an amendment rather than just building it

SPEC.md §11 currently reads, verbatim:

> **Motion:** phase-strip fill + subtle card hover lift only. Respect
> `prefers-reduced-motion`. No gradients, no glassmorphism, no confetti.

"only" is an allowlist of exactly two things. And `CLAUDE.md` line 13 says "UI
must follow §11 exactly — tokens, fonts, phase strip. No generic defaults."
Adding dialog transitions today would mean either violating the spec or quietly
reinterpreting it — and every future plan that says "verify §11 holds" would be
checking against a document nobody believes anymore. Hence: amend it explicitly,
in the open, or don't.

## The constraint the amendment must not break

§11's stated intent is "Dark, dense, calm", and it names the phase strip as
**the** signature element: *"This is the one memorable element; it encodes the
actual sequence, not decoration. Keep everything else quiet."*

That is the real design decision, and it is a good one. If every surface
animates, the phase strip stops being special — the amendment would have
destroyed the thing it was supposed to decorate. So the proposal below is
deliberately a **short allowlist**, not a general permission.

There is also a performance constraint from the 2026-08-06 audit (finding
PERF-01, still unplanned): `src/store.ts`'s `setState` notifies every subscriber
on every patch, so each `log-lines` flush re-renders the whole grid — up to ~10×
per second per running project. **CSS transitions are safe** (they do not re-run
on re-render). **JS-driven animation on cards is not** — it would compound a
known problem. The amendment encodes that distinction.

## Section A — proposed replacement text for §11's Motion bullet

Replace the single Motion bullet with:

```markdown
- **Motion:** restrained and functional — motion exists to explain a state
  change, never to decorate. Allowed:
  - the phase-strip fill (the signature element — it stays the most
    expressive motion in the app, and nothing else may compete with it);
  - a subtle card hover lift;
  - enter/exit transitions on the surfaces that appear over the grid: the
    Add/Edit dialog, the Settings dialog, the log slide-over (which §11
    already calls a *slide*-over), and toasts — fade and/or a short
    translate, ≤200 ms, ease-out;
  - colour/opacity transitions on status pills and phase segments when a
    status actually changes, ≤200 ms;
  - card enter/exit when a project is added or removed.

  Everything else stays still. Still banned: gradients, glassmorphism,
  confetti, parallax, scroll-linked effects, looping/idle animation (the
  `stopping` spinner and the amber pulse on transitional statuses are the
  only loops), and any motion that delays interaction — a control must be
  usable on the frame it appears.

  Implement with **CSS transitions**, not JS animation loops or animation
  libraries. This is a performance requirement, not a style preference: the
  store notifies every subscriber on every log flush, so cards re-render
  frequently; CSS transitions are unaffected by re-render, JS-driven
  animation is not. No new dependency for motion (SPEC.md §4).

  `prefers-reduced-motion` must disable all of the above — the existing
  global rule in `src/index.css` already does this; keep it working.
```

## Section B — proposed clarification for §11's Card contents bullet

The maintainer also asked for "better card design". §11's Card contents bullet
fixes *what is on a card* (name, status pill with port, time slot, primary
button, overflow menu) and the order rule ("no automatic re-sorting, ever").
Those are load-bearing and this proposal does **not** change them.

What is currently ambiguous is whether *visual* refinement is permitted. Proposed
addition to that bullet:

```markdown
  The elements above and their order are fixed; their visual treatment
  (spacing, hierarchy, weight, borders, the exact composition within the
  card) is not, provided it uses the §11 palette tokens and type scale and
  keeps the card readable at a glance in a dense grid.
```

This lets the card be improved without letting it grow new controls, which is
what the fixed list was protecting against.

## What this amendment does NOT authorise

Stated explicitly so a later reader does not over-read it:

- No new card elements or controls (the §11 list stays closed).
- No change to the §11 palette, the three fonts, or the status colours.
- No re-sorting or grouping of cards — that is a separate question (folders),
  deliberately not addressed here.
- No animation library, no new dependency.
- Nothing on the §3 OUT list becomes reachable.

## How to apply

1. Read Section A and B; edit the wording to taste — it is your spec.
2. Apply both to `SPEC.md` §11.
3. Commit on its own, e.g. `Amend SPEC.md §11 to permit restrained functional
   motion`, with a body noting it was a deliberate amendment rather than drift.
4. Only then is UI-polish work in scope. A follow-on implementation plan should
   be written against the amended text, and should treat the allowlist as
   exhaustive.

## Maintenance notes

- If a future request needs motion outside the allowlist, amend §11 again
  deliberately rather than stretching "restrained and functional". The value of
  this section is that it is checkable.
- Reviewer of any follow-on UI plan should verify: no new dependency, CSS
  transitions only, `prefers-reduced-motion` still disables everything, and the
  phase strip is still visually the most expressive motion on screen.
- The audit's PERF-01 (store fan-out re-rendering every card on every log flush)
  remains unplanned. If card motion ever looks janky with a chatty dev server
  running, that finding is the cause, not the CSS.
