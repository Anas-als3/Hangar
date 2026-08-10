# Plan 038: Stop blanking the grid on every mutation

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "refreshRegistryQuietly" src/store.ts`
> Must find the function (unexported) and exactly one call site. On a
> mismatch, STOP.
>
> **Gate ownership**: you run `npm run typecheck`. **Your reviewer runs
> `npm run build` and `cargo check`.** This plan touches no Rust.

## Status

- **Priority**: P2 — the highest-frequency visual event in the app
- **Effort**: S
- **Risk**: LOW — one exported function and nine call-site swaps
- **Depends on**: nothing
- **Category**: bug
- **Planned at**: commit `bb65ea1`, 2026-08-10

## Why this matters

`loadRegistry()` (`src/store.ts:353`) sets `loading: true` before fetching.
`src/App.tsx:218` is a bare `{loading ? <p>Loading…</p> : …}`, so **the entire
grid is replaced by the text "Loading…"** for the duration of every
`get_projects` round trip.

That happens on **nine** paths, all verified at this commit:

| `src/store.ts` | Function | When the user sees it |
|---|---|---|
| :495 | `startProject` (catch) | a rejected Run |
| :606 | `saveNotesAction` | **every notes autosave** |
| :651 | `addProjectAction` | adding a project |
| :667 | `updateProjectAction` | saving an edit |
| :680 | `removeProjectAction` | removing a project |
| :762 | `moveToFolder` | every Move to folder / drag-to-group |
| :788 | `renameFolder` | every folder rename |
| :805 | `ungroupFolder` | every ungroup |
| `src/App.tsx:165` | window `focus` listener | **every time you come back from the browser** |

The notes one is the worst: `NotesPanel` autosaves on an 800 ms typing pause, so
**every pause while writing a note blanks the grid behind the panel**. The
panel's own comment (`NotesPanel.tsx:23-26`) already records that "a save
triggers `loadRegistry()`" — it was written to work around the reload, not to
flag the blanking.

**The fix already exists and is one line from being usable.**
`refreshRegistryQuietly` (`src/store.ts:394`) was added by plan 035 for exactly
this reason — it fetches projects **without** touching `loading`. Its doc
comment says so. It is `async function`, not `export`, and has exactly one call
site (`:445`, on the `running` status event).

Everything below is: export it, and use it in the nine places that should never
have blanked the grid.

## Current state

`src/store.ts:380-402` — the function to export, with the comment that already
explains why it exists:

```ts
/**
 * Plan 035 step 4 — the quiet refresh. Deliberately fetches only `projects`, not the full
 * `loadRegistry()` (which also sets `loading: true` — `App.tsx` swaps the whole grid for
 * "Loading…" while that flag is true, and a mid-run refresh must never blank the grid or the
 * phase strip). Errors are swallowed and the current list is kept: a failed refresh must never
 * clear the grid.
 * ...
 */
async function refreshRegistryQuietly(): Promise<void> {
  try {
    const projects = await getProjects();
    setState({ projects });
  } catch {
    // Swallow: a failed quiet refresh must leave the currently-shown list untouched.
  }
}
```

`src/App.tsx:157-169` — the two effects. **The mount effect must keep
`loadRegistry`**; the focus effect must not:

```tsx
  useEffect(() => {
    void loadRegistry();
  }, []);

  useEffect(() => {
    const onFocus = () => {
      void loadRegistry();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);
```

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run build`, `npm run verify`, `npm run build:app`, `cargo`
anything, or `npm run test:acceptance` — a 600 s no-output watchdog has killed
executor runs here. Keep every Write/Edit under ~60 lines and commit after each.

## Scope

**In scope**:
- `src/store.ts` — export `refreshRegistryQuietly`; swap eight call sites
- `src/App.tsx` — swap the focus listener only

**Out of scope** (do NOT touch):
- **The mount effect at `App.tsx:157-159`.** Startup is the one moment
  "Loading…" is correct: there is no grid yet to blank. Leave it calling
  `loadRegistry`.
- `loadRegistry` itself — its body, its `loading`/`loadError` handling, and its
  `registryError` fetch all stay exactly as they are. It keeps its single
  startup caller.
- The `loading` state field, `App.tsx:218`'s ternary, and the `EmptyState` /
  search-empty branches. This plan removes the *cause*, not the branch.
- Anything under `src-tauri/`, `src/types.ts`, any component other than
  `App.tsx`.
- `NotesPanel.tsx`'s debounce, its Esc path, or its `[notesFor]` effect. A
  separate plan owns the notes data-loss bug and will conflict with you.
- Any new dependency.

## Git workflow

- One commit per step: `Quiet refresh: <what>`.

## Steps

### Step 1: Export the quiet refresh

In `src/store.ts`, add `export` to `refreshRegistryQuietly` and extend its doc
comment with one sentence naming its new role: it is the default refresh for
**every** post-mutation reload, and `loadRegistry` is now the startup-only path.

**Verify**: `npm run typecheck` → exit 0.

### Step 2: Swap the eight store call sites

Replace `await loadRegistry()` with `await refreshRegistryQuietly()` in exactly
these eight functions:

`startProject` (the catch), `saveNotesAction`, `addProjectAction`,
`updateProjectAction`, `removeProjectAction`, `moveToFolder`, `renameFolder`,
`ungroupFolder`.

Two things to preserve exactly:

- **`renameFolder` and `ungroupFolder` call it from a `finally`** (plan 033
  defect 3 put it there so a partial write still refreshes). Keep the `finally`;
  only the function name changes.
- `startProject`'s catch has a comment citing §5's "pathExists must refresh when
  Run is clicked". That still holds — `refreshRegistryQuietly` calls
  `getProjects`, which recomputes `pathExists`. Leave the comment, and add that
  it no longer blanks the grid to do it.

**Verify**: `npm run typecheck` → exit 0; `grep -c "loadRegistry" src/store.ts`
→ the definition and its doc references only, **zero** remaining `await
loadRegistry()` call sites inside those eight functions.

### Step 3: Swap the window-focus listener

In `src/App.tsx`, the **focus** effect calls `refreshRegistryQuietly`. The
**mount** effect keeps `loadRegistry`.

Add a comment on the focus effect explaining the split: coming back from the
browser is the single most frequent event in the app, and blanking the grid to
re-stat three paths is the worst possible trade.

**Verify**: `npm run typecheck` → exit 0.

### Step 4: Self-check

Report each:

- `grep -n "loadRegistry" src/App.tsx` → exactly one call, in the mount effect.
- `grep -n "refreshRegistryQuietly" src/store.ts src/App.tsx` → exported once,
  called nine times (eight in `store.ts` + the `running` handler, one in `App.tsx`).
- `grep -n "finally" src/store.ts` → still two, in `renameFolder` and
  `ungroupFolder`.
- `git status --short` → only the two in-scope files.

**Verify**: `npm run typecheck` → exit 0.

## Test plan

No JS test runner for component behaviour (SPEC.md §4). Manual checks for the
reviewer/maintainer:

- Open Notes on a project and type a paragraph. **The grid behind the panel must
  never flash "Loading…"** — before this it did on every 800 ms pause.
- Drag one card onto another to make a folder → the grid updates in place; no blank.
- Rename a folder, ungroup it, remove a project, save an Edit → same.
- Switch to the browser and back → the grid does not blank. `pathExists` still
  refreshes: move a project's folder away while Hangar is unfocused, come back,
  and the card must still show "Folder not found".
- Launch the app cold → "Loading…" still appears once, which is correct.
- Break the backend (rename `projects.json` to something else while running) and
  come back to the window → the grid keeps showing the old list rather than
  emptying, because the quiet refresh swallows errors. That is intended; confirm
  it does not clear.

## Done criteria

- [ ] `npm run typecheck` exits 0
- [ ] `refreshRegistryQuietly` is exported and used by all eight store paths and
      the window-focus listener
- [ ] `loadRegistry` retains exactly one caller: the mount effect
- [ ] The two `finally` blocks are intact
- [ ] Nothing under `src-tauri/`, no component other than `App.tsx` modified
- [ ] `plans/README.md` status row for 038 updated

## STOP conditions

Stop and report back if:

- Any of the eight functions needs `loading` to be set for a reason you can see
  in the code. Report it rather than leaving that one behind — a partial swap is
  worse than none, because the remaining path becomes the mysterious one.
- The mount effect appears to need the quiet version too. It does not; startup
  has no grid to preserve.
- You find a tenth `loadRegistry` call site. Report it; the table above was
  verified at `bb65ea1` and a tenth means drift.

## Maintenance notes

- **The rule this establishes**: `loadRegistry` is the startup path;
  `refreshRegistryQuietly` is everything else. A new mutation action should use
  the quiet one by default, and reaching for `loadRegistry` needs a reason.
- The `loading` flag now has exactly one producer. If a future feature wants a
  spinner for a slow operation, it should own its own local state rather than
  reusing this global — the global is what made a note keystroke blank the grid.
- `refreshRegistryQuietly` swallows errors by design. That means a genuinely
  broken registry now surfaces only at startup. If that ever matters, the fix is
  a separate error channel, not restoring the blanking.
