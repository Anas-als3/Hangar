# Plan 027: Move the phase strip's `seen` set into the store

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e8b515d..HEAD -- src/store.ts src/components/PhaseStrip.tsx`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P2 — a live bug that blanks the §11 signature element
- **Effort**: S
- **Risk**: LOW — one component, one store field, no backend, no schema
- **Depends on**: nothing
- **Category**: bug
- **Planned at**: commit `e8b515d`, 2026-08-10

## Why this matters

`PhaseStrip` holds its `seen` set in component-local `useState`
(`src/components/PhaseStrip.tsx:42-44`). The set is what distinguishes a
**skipped** phase (dimmed) from a **not yet reached** one (pending) — the whole
point of the §11 signature element. Because it is component state, it dies with
the component.

Unmounting a card is not hypothetical. `ProjectGrid` maps `visibleProjects`
(`src/components/ProjectGrid.tsx:33-35`), which the header search filters
(plans 017/022). Filtering a card out and back **remounts** it.

The worst case is the one that matters most:

- A project is `crashed` after a real run — Pull lit, Install lit, Start lit,
  Ready never reached. The strip shows exactly where it died.
- Type anything in the search box that does not match its name, then clear it.
- The card remounts. The initialiser runs `isPhaseKey("crashed")` → `false` →
  `seen` is **empty** → `reachedIndex` is `-1` → every segment computes
  `lit = false` and `dimmed = i < -1` = `false`.
- All four segments render as **pending**. The strip that existed to explain the
  crash now says nothing, and looks like the project never ran.

The same erasure happens on any remount at `stop-failed`, and mid-run at
`stopping` (also not a `PhaseKey`).

This plan is worth shipping on its own. It is also a prerequisite for anything
that can unmount a card — a folder that collapses, for instance — so it is
deliberately staged first and separately.

## Current state

`src/components/PhaseStrip.tsx:39-62` — the state to move, and the reset rule:

```tsx
export function PhaseStrip({ project }: { project: ProjectView }) {
  const [seen, setSeen] = useState<ReadonlySet<PhaseKey>>(() =>
    isPhaseKey(project.status) ? new Set([project.status]) : new Set(),
  );
  const prevStatus = useRef<Status>(project.status);

  useEffect(() => {
    const previous = prevStatus.current;
    if (previous === project.status) return;
    prevStatus.current = project.status;
    const freshRun =
      (previous === "stopped" || previous === "crashed") &&
      project.status !== "stopped" &&
      project.status !== "crashed";
    setSeen((current) => {
      const base = freshRun ? new Set<PhaseKey>() : current;
      if (!isPhaseKey(project.status)) return base;
      return new Set(base).add(project.status);
    });
  }, [project.status]);
```

`src/store.ts:173-192` — `applyStatusChanged` already computes the identical
"a run is starting" transition, for the log buffer:

```ts
function applyStatusChanged(payload: StatusChangedPayload): void {
  const previous = state.projects.find((p) => p.id === payload.projectId)?.status;
  const projects = state.projects.map((p) =>
    p.id === payload.projectId ? { ...p, status: payload.status } : p,
  );

  const runIsStarting =
    (previous === "stopped" || previous === "crashed") &&
    payload.status !== "stopped" &&
    payload.status !== "crashed";

  setState({
    projects,
    logs: runIsStarting ? { ...state.logs, [payload.projectId]: [] } : state.logs,
  });
```

`src/store.ts:51-68` — the state interface, and `src/store.ts:70-81` — its
initialiser. `logs: Record<string, LogLine[]>` at line 57 is the exact shape to
copy.

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
- `src/store.ts` — one new state field, populated in `applyStatusChanged`
- `src/components/PhaseStrip.tsx` — read the store instead of local state

**Out of scope** (do NOT touch):
- Anything under `src-tauri/`. No Rust, no schema, no persisted field. This set
  is ephemeral view state and must **never** reach `projects.json`.
- `applyStatusChanged`'s existing `runIsStarting` calculation and its log-buffer
  clear. Reuse the value; do not restate, rename, or "tidy" the condition — the
  comment at `store.ts:179-183` explains why it is keyed where it is.
- `ProjectCard.tsx`, `ProjectGrid.tsx`, the phase strip's markup, colours,
  labels, or the `VISIBLE` set. The rendering is correct; only where `seen`
  lives is wrong.
- `logs`, `clearLogs`, or the buffer lifecycle.

## Git workflow

- One commit per file: `Phase strip: <what>`.

## Steps

### Step 1: Add the field to the store

In `src/store.ts`:

1. Add to `HangarState` (after `logs`, matching its comment style):

   ```ts
   /** Phases actually observed per project this run (plan 027) — ephemeral view state that
    *  outlives a card unmount, so search-filtering a card out and back cannot blank the §11
    *  phase strip. Never persisted: this is not a `Project` field. */
   phasesSeen: Record<string, string[]>;
   ```

   Use `string[]`, not `Set`. `setState` spreads state and every subscriber
   re-renders through `useSyncExternalStore`; a plain array keeps the snapshot
   comparable and serialisable. `PhaseStrip` does membership tests on at most
   four items.

2. Initialise it to `{}` in the `state` literal (`store.ts:70-81`).

3. In `applyStatusChanged`, extend the **existing** `setState` call — do not add
   a second one. Reuse the `runIsStarting` value already computed:

   - The base for this project is `[]` when `runIsStarting`, otherwise its
     current array (or `[]` when absent).
   - If `payload.status` is one of `updating` / `installing` / `starting` /
     `running`, append it when not already present.
   - Otherwise store the base unchanged.

   Keep the phase-key check local to `store.ts`; do not import from
   `PhaseStrip.tsx` (the dependency runs the other way).

**Verify**: `npm run typecheck` → exit 0.

### Step 2: Read the store from `PhaseStrip`

In `src/components/PhaseStrip.tsx`:

1. Delete the `useState`, the `prevStatus` ref, and the whole `useEffect`. The
   store now owns every one of those responsibilities.
2. Read the array for this project from the store and build the lookup for
   `seen.has(key)`. Follow how other components subscribe — check
   `src/components/LogPanel.tsx` for the existing hook and use the same one; do
   not invent a second subscription mechanism.
3. Leave `reachedIndex`, the `PHASES` map, `VISIBLE`, every class string and the
   negative-margin comment **exactly** as they are.

One behaviour deliberately changes and must be preserved as described: a status
the store never saw is no longer inferred from the current `project.status` at
mount. That is the point — the store observes every `status-changed` event from
app start, so it is strictly better informed than a freshly mounted component.

**Verify**: `npm run typecheck` → exit 0; `npm run build` → exit 0.

### Step 3: Confirm the erasure is gone, structurally

You cannot drive the GUI. Verify by reading and report each:

- `grep -n "useState\|useRef\|useEffect" src/components/PhaseStrip.tsx` — none
  of the three remain.
- `grep -c "setState" src/store.ts` — unchanged from before your edit (report
  the before and after numbers). A second write per status change is a
  regression.
- Show `applyStatusChanged` in full in your report so the reviewer can confirm
  `runIsStarting` is computed once and used for both `logs` and `phasesSeen`.

**Verify**: `git status --short` shows only the two in-scope files.

## Test plan

There is no JS test runner in this repo (SPEC.md §4's dependency rule), so no
automated test. Manual checks for the reviewer/maintainer:

- Run a project, let it reach `running`, then Stop it. Type a non-matching
  string in the search box, clear it. The strip must return **identical** — not
  blank.
- Force a crash (a project whose dev command exits non-zero). Confirm the strip
  shows where it died, then search-filter it out and back. It must still show
  where it died. **This is the bug; this is the check that matters.**
- Run the same project again. The strip must reset to empty and re-light from
  Pull, exactly as it does today.
- A project that has never run shows no strip at all (`stopped` is not in
  `VISIBLE`).

## Done criteria

- [ ] `npm run typecheck` exits 0
- [ ] `npm run build` exits 0
- [ ] `PhaseStrip.tsx` contains no `useState`, `useRef` or `useEffect`
- [ ] `setState` call count in `store.ts` is unchanged
- [ ] `phasesSeen` appears nowhere under `src-tauri/` and in no persisted type
- [ ] Only `src/store.ts` and `src/components/PhaseStrip.tsx` modified
- [ ] `plans/README.md` status row for 027 updated

## STOP conditions

Stop and report back if:

- Making this work seems to require a `Project` field, a §7 command, or any
  backend change. It does not — this is ephemeral view state, and persisting it
  would put render bookkeeping in the user's registry file.
- You cannot reuse `applyStatusChanged`'s existing `runIsStarting` value and
  need a second `setState`. Report the constraint rather than adding a write.
- You conclude the strip should keep seeding itself from `project.status` at
  mount as a fallback. It should not — that seeding **is** the bug, and a
  fallback would silently restore it for `crashed`.

## Maintenance notes

- The reset rule now lives in exactly one place. If SPEC.md's definition of "a
  run is starting" ever changes, `applyStatusChanged` is the only edit, and the
  log buffer and the phase strip cannot drift apart.
- Anything that unmounts a card — search, a future folder that collapses, a
  virtualised grid — is now safe for the phase strip. Uptime was already safe:
  `useCoarseNow` re-seeds on mount and `uptimeLabel` derives from the persisted
  `lastRunAt` (`ProjectCard.tsx:91-114`).
- `plans/README.md`'s gap-analysis item about card-local state losing data on
  unmount should be corrected when this ships: the phase strip was the real
  case; uptime was not.
