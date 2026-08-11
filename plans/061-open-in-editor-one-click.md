# Plan 061: Open in editor, in one click

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise. Update this plan's row in
> `plans/README.md` when done, unless a reviewer told you they maintain it.
>
> **Drift check**: `grep -n "Open in editor" src/components/ProjectCard.tsx && grep -n "pub async fn open_in_editor" src-tauri/src/commands.rs`
> Both must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. Your reviewer runs `npm run build` and the bundle.

## Status

- **Priority**: P1 — directly requested by the maintainer
- **Effort**: **S**
- **Risk**: LOW on the backend (nothing changes there), MED on §11 — this adds
  an element to a card, and §11 says that list is fixed
- **Depends on**: nothing
- **Category**: feature
- **Planned at**: 2026-08-11

## What is actually being asked for

The maintainer: *"I want to add new feature that with one click u can edit the
project, like open it with editor directly with one click."*

**The backend already does this.** `open_in_editor` exists in §7, is implemented
in `commands.rs`, and runs through `run::open_in_editor`. `api.ts` exports
`openInEditor`. It works today.

The problem is purely where it lives: `ProjectCard.tsx:109` puts it inside the
overflow menu, so reaching it costs **two clicks and a menu scan** — open `⋯`,
find the right row among seven, click it.

So this plan **adds no capability**. It moves an existing one into reach. Say
that plainly in your report rather than describing it as a new feature; the
whole change is a button and a §11 amendment.

## The §11 problem, and how to resolve it

§11 says: *"Otherwise the elements above and their order are fixed."* Adding a
second visible control to every card is a change to that list, so it needs an
amendment — and the amendment has to answer the objection §11 raises itself.

That objection is real and specific. §11 repeats, twice, that nothing may push
the **primary Run/Stop button** out of view or occlude it, because *"a dense
grid that has to be scrolled to find the button has lost the plot"* — and on a
`stop-failed` card that button is the only route out of the state.

So the amendment must state, and the implementation must honour:

- The editor control is **secondary and quiet** — it never competes with Run
  for weight, colour, or size. **It does not use the accent colour**; the accent
  belongs to Run and the active phase, and §11's palette is closed.
- It **shares the row that already holds `⋯`**, so the card gains no new row and
  **no height**. A card's silhouette must not change — §11 already made that
  argument once, for the phase strip, with measurements.
- It is **icon-only with an accessible label** (`aria-label` + `title`), so it
  costs almost no width. No text label; "Open in editor" beside "Run" reads as
  two equal choices, and they are not.
- **The overflow-menu row stays.** Do not remove it. Discoverability lives in
  the menu; speed lives on the card. Removing the row would also break the
  muscle memory of the only user this app has.

## Scope

**In scope**:
- `src/components/ProjectCard.tsx` — the button.
- `SPEC.md` §11 — the amendment.
- `src/components/ProjectCard.test.mjs` **or** a zero-import leaf if any logic
  is worth extracting — see "Testing" below.

**Out of scope** (do NOT build):
- **Any backend change at all.** `commands.rs`, `run.rs`, `process.rs`, §7, §6,
  §8, §9 are untouched. `git diff --stat` must show no `.rs` file.
- Any change to what "open in editor" *does*, including which editor, argument
  handling, or the settings field that stores the command.
- A second control for anything else (browser, folder, terminal). One button.
- Any change to the overflow menu's contents or order.
- Any new dependency, icon library, or SVG sprite system. If you need a glyph,
  draw it inline with the same approach the codebase already uses — check how
  the existing header buttons and the `⋯` control render, and match them.

## The states it must get right

Read `ProjectCard.tsx` for how the existing controls handle these, and match:

1. **`pathExists === false`** — §12 disables Run when the folder is gone.
   Opening an editor on a missing folder is the same category of nonsense.
   **Disable it**, with a `title` saying why.
2. **Every §6 status.** Unlike Run, this control is *status-independent* —
   opening the source of a running project is exactly as valid as opening a
   stopped one. **It must never be disabled by status, and it must never be
   confused for a §6 action.** It does not touch the state machine.
3. **A slow or failing editor launch.** The existing menu action already routes
   errors to the §7 toast. Reuse the exact same call path — do not add a second
   error route.

## Testing

The button is presentation, and this repo does not have a React test harness —
do **not** add one for this. Instead:

- If you extract any decision (e.g. "is this control disabled?") into a pure
  function, put it in a **zero-import leaf module** with a `node --test` file,
  matching `src/launchLine.ts` and `src/session.ts` exactly. That is the
  repo's established pattern for testable frontend logic.
- If the change is genuinely just JSX with no branching worth naming, **say so
  and add no test** rather than writing a test that asserts a class name.
  Report which you chose.

The Rust test count **must not change**. If it does, you have touched the
backend, which is out of scope.

## Steps

1. **Amend §11** with the paragraph described above.
   **Verify**: `grep -n "one click" SPEC.md` finds it in §11.
2. **Add the button** in `ProjectCard.tsx`, in the `⋯` row, icon-only, quiet,
   calling the same `openInEditorAction` the menu row calls.
3. **Self-check**: `git diff --stat` shows **no `.rs` file**; the overflow menu
   still contains its "Open in editor" row; no new dependency in `package.json`.

Verify after each: `cargo check --all-targets` → 0; `cargo test` → **identical
count**; `npx tsc --noEmit` → 0.

## Done criteria

- [ ] Three gates green; `cargo test` count **unchanged**
- [ ] `git diff --stat` contains no `.rs` file — paste it
- [ ] The card's **height is unchanged** in every status — the button shares the
      `⋯` row and adds no line
- [ ] The overflow menu's "Open in editor" row still exists
- [ ] The button is disabled when `pathExists === false`, and **not** disabled
      by any §6 status
- [ ] It does not use the accent colour
- [ ] §11 amended; `plans/README.md` row updated

## STOP conditions

- You find yourself editing any `.rs` file. The backend is done; this is a UI
  change only.
- The button needs a new row, or changes card height in any status.
- You are tempted to also add "open folder" / "open terminal" while in there.
  One button. A second one is a separate decision and a separate amendment.
- Making it work requires the accent colour to be visible enough. If a quiet
  treatment cannot be found within §11's closed palette, **stop and report** —
  that is a design question for the maintainer, not a licence to add a colour.

## Maintenance notes

- The pressure this will attract is a **row of icon buttons** on every card —
  editor, then folder, then terminal, then GitHub. Each is individually
  reasonable and the sum is a toolbar on a card that is meant to be scanned at a
  glance. The overflow menu exists precisely so that the card does not become
  that. **One promoted control is the budget**; promoting a second one means
  demoting this one, not adding to it.
- The reason this one earns promotion is that it is the action a developer takes
  *most often and most repeatedly*: you open a project's code many times a day,
  and you Run it a handful of times.
