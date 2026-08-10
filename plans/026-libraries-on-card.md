# Plan 026: Show the detected libraries on the card, not only in the Edit dialog

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat 5cf6a25..HEAD -- src/components/ProjectCard.tsx src/components/AddEditDialog.tsx`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW-MED — the risk is entirely layout: a compact card can be broken
  by one line too many, and no gate in this repo renders a component.
- **Depends on**: plans/023 (DONE — detection), plans/025 (DONE — backfill)
- **Category**: direction (maintainer-requested)
- **Planned at**: commit `5cf6a25`, 2026-08-10

## Why this matters

Plan 023 detects each project's framework and its notable libraries, but only
the framework reaches the card. The libraries are behind Edit. The maintainer's
words: *"the react and other stuff should show too on the card no need to go in
edit to see them."*

SPEC.md §11's Card contents bullet was extended on 2026-08-10 to permit exactly
this — read it before starting, it constrains the design.

## The constraint that governs this plan

Cards are **compact**: the grid track is `minmax(14rem,1fr)` and the card shell
is `p-3` (plan 019, chosen by the maintainer over a roomier option). §11's
amended text is explicit:

> The libraries line is **capped** — at most the first few fit a 14 rem card,
> and the rest are indicated by a count (`+3`), with the full list remaining in
> the Edit dialog. It must never wrap to a second line or push the primary
> Run/Stop button out of view; a dense grid that has to be scrolled to find the
> button has lost the plot.

A real project can produce many entries — plan 023's allow-list has 19 members.
Rendering them all at 14 rem would wrap to three lines and shove the Run button
down. **The cap is the feature**, not an implementation detail.

## Current state

`src/components/ProjectCard.tsx`, the status row (~lines 236-274):

```tsx
      <div className="flex items-center gap-2 text-sm">
        <span className={`inline-flex items-center gap-2 rounded-full bg-white/5 px-2.5 py-1 font-medium transition-colors duration-200 ${STATUS_TONE[project.status]}`}>
          <span aria-hidden="true" className="size-1.5 rounded-full bg-current" />
          <span>{STATUS_LABEL[project.status]}</span>
          {/* port: a button when running, inert span otherwise */}
        </span>

        {/* §11 (added 2026-08-09): the one permitted stack badge */}
        {project.stack?.framework && (
          <span className="rounded-full border border-white/10 bg-white/5 px-2 py-0.5 font-mono text-xs text-muted">
            {project.stack.framework}
          </span>
        )}

        {!project.pathExists && (
          <span className="rounded-full bg-status-danger/10 px-2.5 py-1 text-xs text-status-danger">
            Folder not found
          </span>
        )}
      </div>
```

Card layout order today: `<header>` (name, path, overflow menu) → status row (above)
→ time slot (`lastRunLabel` / uptime) → `<footer>` (Run/Stop button, command) →
`<PhaseStrip>`.

`src/components/AddEditDialog.tsx` already renders the full list read-only
beneath the path, using `stack.libraries.join(" · ")` plus a relative
`detectedAt`. That stays — it is the overflow destination.

`project.stack` is `{ framework?: string; libraries: string[]; detectedAt: string }`
(SPEC.md §5). `libraries` is always present, possibly empty.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run verify`, `npm run build`, `npm run build:app`,
`cargo test`, or `npm run test:acceptance` — a 600 s no-output watchdog has
killed executor runs here. This plan touches no Rust at all. Keep every
Write/Edit under ~60 lines and commit after each. Your reviewer runs the full suite.

## Scope

**In scope**:
- `src/components/ProjectCard.tsx` — the libraries line only

**Out of scope** (do NOT touch):
- Any Rust, anything under `src-tauri/`. Detection is done and correct; this is
  purely display.
- `src/components/AddEditDialog.tsx` — its full list is the overflow
  destination and must keep working unchanged.
- The stack badge, the status pill, the port button, the phase strip, the
  overflow menu — all existing card elements stay exactly as they are.
- `src/store.ts`, the persisted schema, any command.
- Any new dependency.

## Git workflow

- One commit: `Card: show detected libraries with a capped overflow count`.

## Steps

### Step 1: The capped line

In `ProjectCard.tsx`, render a single line beneath the status row when
`project.stack?.libraries` is non-empty.

Requirements:
- Show at most **3** entries, joined with ` · `.
- If more exist, append a quiet `+N` where N is the remainder. Example with 6
  libraries: `React · Tailwind · tRPC +3`.
- Apply `truncate` so a long trio still cannot wrap — the cap protects the
  common case, `truncate` protects the pathological one (a project with three
  long names). Both are needed.
- Give the element a `title` containing the **full** list, so hovering reveals
  everything the card cannot show. This is the same affordance the path and
  command lines already use.
- Style quietly: `text-xs text-muted`, and `font-mono` only if it reads better
  than the sans default at that size — judgement, but it must be visibly
  subordinate to both the status pill and the framework badge. Use existing
  tokens; **no raw hex** (a reviewer greps for it).
- Render nothing at all when `libraries` is empty. No "no libraries detected"
  placeholder — that is noise on every non-Node project.

Place it **between the status row and the time slot**. Do not put it inside the
status row's flex container: that row already holds the pill, the badge and the
"Folder not found" warning, and adding a fourth item there is what makes a
14 rem card wrap.

**Verify**: `npm run typecheck` → exit 0.

### Step 2: Confirm the layout still holds

You cannot see the UI, so verify structurally instead and report each:

- `grep -c "truncate" src/components/ProjectCard.tsx` — the new line is included.
- The libraries element is a sibling of the status row `<div>`, not a child.
  Show the surrounding JSX in your report so the reviewer can confirm.
- The `<footer>` containing the Run/Stop button is still the last flex child
  before `<PhaseStrip>`, unchanged.

**Verify**: `npm run typecheck` → exit 0; `git status --short` shows only
`src/components/ProjectCard.tsx`.

## Test plan

No automated test — there is no JS test runner (SPEC.md §4's dependency rule),
and no gate in this repo renders a component. This is exactly the class of
change that shipped a full-screen modal on 2026-08-09 while every check passed,
so reason carefully about what actually renders.

Manual checks for the reviewer/maintainer:
- A project with 1-3 libraries shows them all, no `+N`.
- A project with more shows the first three and a correct `+N`.
- Hovering the line reveals the full list.
- A project with no `package.json` shows no line at all.
- At the default window width the card does not grow taller than before by more
  than one line, and the Run button is still visible without scrolling.
- The Edit dialog still shows the complete list.

## Done criteria

- [ ] `npm run typecheck` exits 0
- [ ] At most 3 libraries render, with `+N` for the remainder
- [ ] The line carries a `title` with the full list and cannot wrap
- [ ] `grep -rnE "#[0-9A-Fa-f]{6}" src/components/ProjectCard.tsx` → no matches
- [ ] Only `src/components/ProjectCard.tsx` modified
- [ ] `plans/README.md` status row for 026 updated

## STOP conditions

Stop and report back if:

- Fitting the line requires changing the grid track, the card padding, or
  removing any existing card element. §11 fixes the element list; the maintainer
  chose the compact density deliberately. Report instead.
- You want to make the line interactive (clickable library chips, a popover).
  §11 says these elements are display-only, never controls.
- You conclude 3 is the wrong cap. It might be — but changing it is a judgement
  the maintainer should make by looking at a real card, so ship 3 and say in
  your report that you think it should differ.

## Maintenance notes

- The cap and the Edit dialog are a pair: the card shows enough to recognise a
  project at a glance, Edit shows everything. If the card ever gains room (a
  wider grid track), revisit the cap rather than letting the line grow.
- Plan 023's allow-list has 19 entries, so `+N` will be common on real projects.
  If it is *always* showing `+N` on every card, the allow-list is too broad for
  card display and the right fix is a smaller "show on card" subset, not a
  bigger cap.
