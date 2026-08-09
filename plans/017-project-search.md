# Plan 017: Filter the project grid by name with a search field

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat fe734e4..HEAD -- src/App.tsx src/store.ts src/components/ProjectGrid.tsx`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: direction (maintainer-requested feature)
- **Planned at**: commit `fe734e4`, 2026-08-09
- **Requested by**: the maintainer, 2026-08-09

## Why this matters

Requested directly. It is also the cheapest of the requested features and the
only one needing neither a spec amendment nor a storage change: filtering hides
non-matching cards, which is **not** re-sorting, so SPEC.md §11's "Cards render
in `projects.json` array order; … no automatic re-sorting, ever" continues to
hold exactly as written.

Honest scoping note for whoever picks this up: its value scales with project
count, and the registry currently holds one project. It is worth building
because it was asked for and it is genuinely small — not because there is
evidence of the friction it solves. SPEC.md §15 test 9 is the mechanism meant to
supply that evidence.

## Current state

- `src/components/ProjectGrid.tsx` is 18 lines and takes the already-filtered
  list as a prop — it does no selection of its own:

```tsx
export function ProjectGrid({ projects }: { projects: ProjectView[] }) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(20rem,1fr))] gap-4">
      {projects.map((project) => (
        <ProjectCard key={project.id} project={project} />
      ))}
    </div>
  );
}
```

Its header comment says "Cards render in `projects.json` array order. No
sorting, ever." Preserve that — filtering must not reorder.

- `src/store.ts` holds `HangarState` (fields: `projects`, `registryError`,
  `loading`, `loadError`, `logs`, `openLogsFor`, `toast`, `dialog`) with a
  `setState` that shallow-merges a patch and notifies every subscriber. Follow
  the existing action style exactly (small exported functions calling
  `setState`).
- `src/App.tsx` renders the header (with the Add button and Settings gear), the
  empty state, and `<ProjectGrid projects={...} />`.
- Styling tokens live in `src/index.css`: `bg-surface`, `bg-bg`, `text-text`,
  `text-muted`, `bg-accent`, `border-white/10`, fonts `font-display` /
  `font-mono`. **No raw hex anywhere** — a reviewer greps for it.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |
| Full verify | `npm run verify` | exit 0 |

`cargo` needs `PATH="$HOME/.cargo/bin:$PATH"` if a command reaches for it.
**Do not run** `npm run build`, `cargo test`, or `npm run test:acceptance` — a
600 s no-output watchdog has killed six executor runs on this repo. Keep every
Write/Edit under ~60 lines and commit after each one; a reviewer runs the full
suite.

## Scope

**In scope**:
- `src/store.ts` (a `search` field + one action)
- `src/App.tsx` (the input, and passing the filtered list)
- `src/components/ProjectGrid.tsx` (only if an empty-results state belongs there)

**Out of scope** (do NOT touch):
- Any Rust. Filtering is a pure view concern; the backend must not learn about it.
- `src/components/ProjectCard.tsx` — no card changes in this plan.
- Sorting, grouping, folders, or filtering by anything other than name. Type/
  framework filtering was deliberately deferred (nothing persists a framework
  today) and folders are a separate, larger design question.
- Persisting the search term to `projects.json` or settings — it is ephemeral
  UI state, and adding a field would trigger SPEC.md §16's parked versioned
  storage wrapper for no reason.
- Adding any dependency.

## Git workflow

- One commit per file: `Add project search: <what>`.

## Steps

### Step 1: Store state and action

Add `search: string` to `HangarState` (initial `""`) and an exported
`setSearch(value: string)` action in the existing style. Nothing else.

**Verify**: `npm run typecheck` → exit 0.

### Step 2: The filter itself

Add an exported pure helper — put it in `src/store.ts` beside the state so it is
importable and unit-testable later:

```ts
/** Case-insensitive substring match on name, order preserved (SPEC.md §11). */
export function filterProjects(projects: ProjectView[], search: string): ProjectView[] {
  const q = search.trim().toLowerCase();
  if (q === "") return projects;
  return projects.filter((p) => p.name.toLowerCase().includes(q));
}
```

Match on `name` only. Do NOT match on `path` or `command` — the request was
"searching for projects by name", and matching hidden fields makes results
inexplicable ("why did that card match?").

`filter` preserves array order by construction, which is what keeps §11's
ordering rule true.

**Verify**: `npm run typecheck` → exit 0.

### Step 3: The input in the header

In `src/App.tsx`, add a search input to the existing header row, left of the Add
button. Requirements:

- Placeholder `Search projects`.
- Controlled by `search` from the store via `setSearch`.
- Styled with the existing tokens — mirror the input styling already used in
  `src/components/SettingsDialog.tsx` (`rounded-md border border-white/10 bg-bg
  px-3 py-2 text-sm text-text outline-none focus:border-accent`). No raw hex.
- `type="search"`, and an `aria-label` since there is no visible label.
- Escape clears it (a small nicety consistent with Esc closing the slide-over
  and dialogs; keep the handler local to the input, do not add a global listener).

Render `<ProjectGrid projects={filterProjects(projects, search)} />`.

**Verify**: `npm run typecheck` → exit 0.

### Step 4: Empty-results state

When the registry is non-empty but the filter matches nothing, show a quiet line
in the grid area — e.g. `No projects match "<term>".` — using `text-muted`. Do
**not** reuse or alter the §11 first-run empty state ("No projects yet. Add your
first one." + Add button); they mean different things and conflating them would
tell a user with 10 projects that they have none.

Hide the search input entirely when the registry is empty — there is nothing to
search, and it would sit next to the first-run empty state looking broken.

**Verify**: `npm run typecheck` → exit 0.

### Step 5: Gates and commit

**Verify**: `npm run verify` → exit 0; `git status --short` shows only in-scope
files.

## Test plan

No automated test is required by this plan, and note honestly why: there is no
JS test runner (SPEC.md §4's dependency rule; see the amendment recorded in
`plans/README.md`'s rejected-findings section). `filterProjects` is written as an
exported pure function specifically so it is ready for one if a runner is ever
added.

Manual checks for the reviewer or maintainer, since a subagent cannot drive the
GUI:
- Typing part of a project name narrows the grid; clearing restores every card.
- Matching is case-insensitive.
- With two or more projects, filtering does not change their relative order.
- A non-matching term shows the empty-results line, not the first-run empty state.
- With zero projects registered, the search input is not rendered.

## Done criteria

- [ ] `npm run verify` exits 0
- [ ] `grep -rnE "#[0-9A-Fa-f]{6}" src/App.tsx` → no matches (tokens only)
- [ ] `filterProjects` is exported and pure (no store access inside it)
- [ ] No Rust file modified; no dependency added
- [ ] `plans/README.md` status row for 017 updated

## STOP conditions

Stop and report back if:

- Implementing this appears to need a backend command or a change to
  `projects.json` — it must not; filtering is pure view state.
- You find yourself sorting, grouping, or reordering the list for any reason —
  SPEC.md §11 forbids automatic re-sorting outright.
- The header has no room for the input without restructuring `ProjectCard` or
  the grid layout — report the constraint rather than redesigning the header.

## Maintenance notes

- If filtering by framework/type is picked up later, it needs a decision first:
  nothing persists a framework today (`read_package_json` sniffs `next`/`vite`/
  `react-scripts` for the port suggestion and discards it), so it means either a
  new persisted field — which promotes SPEC.md §16's versioned storage wrapper —
  or re-sniffing at load, which adds filesystem reads to startup.
- Folders, if built, will interact with this directly: the natural behaviour is
  that search spans all folders. Decide that explicitly rather than inheriting it.
- Reviewer should scrutinise: order preservation, and that the two empty states
  stay distinct.
