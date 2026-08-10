# Plan 039: Stop discarding notes on Esc, and make dialogs typeable

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "closeNotes()" src/components/NotesPanel.tsx && grep -c "autoFocus" src/components/AddEditDialog.tsx`
> The first must match; the second must be `0`. On a mismatch, STOP.
>
> **Gate ownership**: you run `npm run typecheck`. **Your reviewer runs
> `npm run build` and `cargo check`.** This plan touches no Rust.

## Status

- **Priority**: P1 for step 1 (data loss), P2 for the rest
- **Effort**: S per fix, M in total
- **Risk**: LOW-MED — the `inert` step changes global focus behaviour
- **Depends on**: plan 038 (both touch `src/App.tsx`; 038 must merge first)
- **Category**: bug
- **Planned at**: commit `bb65ea1`, 2026-08-10

## Why this matters

### Step 1 — notes typed then Esc-closed are silently destroyed

`src/components/NotesPanel.tsx` autosaves on an 800 ms debounce
(`handleChange` sets `debounceRef`). Two of the three close routes flush it:

- **Click-away** and the **✕ button** fire `mousedown` first, so the textarea
  blurs, `handleBlur` clears the timer and calls `save(value)` when dirty.
- **Esc** (`:36-43`) calls `closeNotes()` directly. No blur fires. The
  `[notesFor]` effect then re-runs, hits
  `if (debounceRef.current) clearTimeout(debounceRef.current)`, and **everything
  typed since the last save fire is gone.**

Esc is the route §11 documents (*"Esc closes the slide-over"*), so the
documented way to close the panel is the one that loses your work. That is data
loss, not friction — and notes are a feature nobody has adopted yet, which makes
this the worst possible first experience of it.

### Steps 2-4 — every dialog opens somewhere you cannot type

Verified at this commit:

- **No dialog focuses its input on open.** `grep -c autoFocus` is `0` in
  `AddEditDialog.tsx`, `SettingsDialog.tsx` and `NotesPanel.tsx`. The only
  `autoFocus` in the repo is `MoveToFolderDialog.tsx:166`, and it is conditional
  on the "New folder…" field. So opening Add, Edit, Settings or Notes means:
  open, type, nothing happens, click, type again.
- **No dialog is a `<form>`**, so **Enter never saves** anywhere. Save is
  click-only in all four.
- **All four set `aria-modal="true"`** (`NotesPanel:89`, `MoveToFolder:117`,
  `AddEdit:210`, `Settings:58`) **with no focus trap**, so Tab walks straight
  out of the dialog and onto the live Run buttons behind the backdrop. The ARIA
  attribute is a promise the DOM does not keep.

For a maintainer with ADHD, "open, type, discover nothing happened, re-aim,
type again" is a ritual tax on the most common interaction in the app.

## Current state

`src/components/NotesPanel.tsx:36-43` — the Esc path that loses text:

```tsx
  // §11: Esc closes the slide-over — the only keyboard shortcut in v0.
  useEffect(() => {
    if (!notesFor) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") closeNotes();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [notesFor]);
```

`:64-77` — the debounce and the blur flush that Esc bypasses:

```tsx
  function handleChange(text: string): void {
    setValue(text);
    setDirty(true);
    setJustSaved(false);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => save(text), SAVE_DEBOUNCE_MS);
  }

  function handleBlur(): void {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (dirty) save(value);
  }
```

`src/components/SettingsDialog.tsx:17-28` — settings load **once, on mount**,
with `[]` deps, so cancelling an edit and reopening shows the abandoned value
rather than what is on disk.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run build`, `npm run verify`, `npm run build:app`, `cargo`
anything, or `npm run test:acceptance`. Keep every Write/Edit under ~60 lines
and commit after each.

## Scope

**In scope**:
- `src/components/NotesPanel.tsx` — the Esc flush, autofocus
- `src/components/AddEditDialog.tsx` — autofocus, Enter-to-save
- `src/components/SettingsDialog.tsx` — autofocus, Enter-to-save, refetch on open
- `src/components/MoveToFolderDialog.tsx` — Enter-to-save
- `src/App.tsx` — the `inert` step only

**Out of scope** (do NOT touch):
- Anything under `src-tauri/`, `src/types.ts`, `src/store.ts`.
- `NotesPanel`'s 800 ms debounce value, its `[notesFor]` seeding effect's
  deliberate dependency choice (its comment explains why it is not keyed on
  `project`), or `saveNotesAction`.
- `loadRegistry` / `refreshRegistryQuietly` — plan 038 owns those and must
  already be merged.
- The Esc **close** behaviour itself. Esc still closes; it just flushes first.
- `ProjectCard.tsx`, `ProjectGrid.tsx`, `FolderTile.tsx`, the drag subsystem.
  The card menu's and folder band's Esc handling was settled by plan 033 and
  must keep working.
- Adding a focus-trap library, or hand-rolling a Tab-cycling trap. Step 4 uses
  `inert` and nothing else.
- Any new dependency.

## Git workflow

- One commit per step: `Dialog input: <what>`.

## Steps

### Step 1: Flush pending notes before Esc closes — **the important one**

In `NotesPanel.tsx`, the Esc handler must flush any pending save **before**
calling `closeNotes()`.

Guidance:

- Reuse the existing flush logic rather than duplicating it: `handleBlur`
  already does exactly "clear the timer, save if dirty". Extract it into a named
  `flushPendingSave()` and call it from both `handleBlur` and the Esc handler.
- The handler is inside a `useEffect` with `[notesFor]` deps and closes over
  `dirty` and `value`. **A stale closure here silently reintroduces the bug.**
  Either add the needed deps, or hold the current text in a ref that the effect
  reads — pick one and say in your report which, and why it cannot go stale.
- Do not make Esc await the save. `save()` is fire-and-forget elsewhere in this
  file; closing should not wait on a round trip.

**Verify**: `npm run typecheck` → exit 0.

### Step 2: Focus the thing the user is about to type in

Add `autoFocus` to the primary input of each dialog:

- `NotesPanel.tsx` — the textarea
- `AddEditDialog.tsx` — the Name field
- `SettingsDialog.tsx` — the editor-command field

Leave `MoveToFolderDialog.tsx:166`'s existing conditional `autoFocus` exactly as
it is.

If `autoFocus` does not take because the element mounts inside a conditional
branch, use a ref + a `useEffect` that focuses on open — do **not** reach for a
timeout.

**Verify**: `npm run typecheck` → exit 0.

### Step 3: Enter saves

Wrap each dialog's fields in a `<form>` whose `onSubmit` calls the existing save
handler, with `event.preventDefault()`. Make the Save button `type="submit"` and
every other button in the dialog explicitly `type="button"` — an unmarked
`<button>` inside a form defaults to submit, so Cancel would save.

Constraints:

- **Do not bypass the existing disabled/validation state.** `AddEditDialog`'s
  `canSave` and `SettingsDialog`'s `saving` guard must still gate submission —
  the handler should early-return exactly as the click path does.
- **`NotesPanel` gets no form.** Its textarea is multi-line prose; Enter must
  insert a newline. It already autosaves. Leave it click-and-blur only.
- `MoveToFolderDialog`'s new-folder text field is the case that most wants this
  — typing a name and pressing Enter should move.

**Verify**: `npm run typecheck` → exit 0.

### Step 4: Make `aria-modal` true in fact

While any dialog or slide-over is open, the rest of the app must not be
reachable by Tab.

- In `src/App.tsx`, apply the `inert` attribute to the main content wrapper
  (header + `<main>`) whenever an overlay is open — derive that from the store's
  existing `dialog`, `openLogsFor` and `notesFor`, exactly the three the folder
  band's Esc guard already consults.
- React 18 does not have a typed `inert` prop. Set it via a ref and
  `setAttribute`/`removeAttribute` in an effect, or use `{...{ inert: "" }}` —
  whichever typechecks cleanly. **Do not add `// @ts-expect-error`** and do not
  loosen `strict`.
- The dialogs render **outside** that wrapper (they are siblings at the end of
  `App`'s tree). Confirm that by reading before you wrap — if any overlay is
  inside the wrapper, `inert` would disable the dialog itself, and you must
  report rather than restructure the tree.

**Verify**: `npm run typecheck` → exit 0.

### Step 5: Settings shows what is on disk

`SettingsDialog`'s load effect has `[]` deps, so it fetches once per app
lifetime. Open Settings, type a wrong command, press Cancel, reopen → the wrong
command is still there.

Re-fetch when the dialog opens. Keep the existing `cancelled` flag pattern.

**Verify**: `npm run typecheck` → exit 0.

### Step 6: Self-check

Report each:

- `grep -n "flushPendingSave" src/components/NotesPanel.tsx` → defined once,
  called from both `handleBlur` and the Esc handler.
- `grep -c "autoFocus" src/components/NotesPanel.tsx src/components/AddEditDialog.tsx src/components/SettingsDialog.tsx` → 1 each.
- `grep -n "type=\"submit\"\|type=\"button\"" src/components/*.tsx` → every
  button inside a new `<form>` carries an explicit type.
- `grep -n "inert" src/App.tsx` → present, driven by the three overlay fields.
- `grep -n "<form" src/components/NotesPanel.tsx` → **no match**.
- `git status --short` → only the five in-scope files.

**Verify**: `npm run typecheck` → exit 0.

## Test plan

No JS test runner for component behaviour (SPEC.md §4). Manual checks for the
reviewer/maintainer — **the first one is the plan's reason for existing**:

- Open Notes, type a sentence, press **Esc within 800 ms**, reopen the panel.
  **The sentence must be there.** Before this it was gone.
- Same with click-away and with ✕ — both must still work as they did.
- Open Add → the cursor is in Name. Type a name and press Enter → it saves (or
  stays put if the form is incomplete, exactly as the disabled button behaves).
- Open Settings → cursor in the editor command. Change it, press **Cancel**,
  reopen → the **saved** value is shown, not the abandoned one.
- With any dialog open, press Tab repeatedly → focus stays inside the dialog and
  never lands on a Run button behind the backdrop.
- Open a folder band, then a member's `⋯` menu, press Esc → the menu closes and
  the folder stays open (plan 033's behaviour must be unchanged).
- In Notes, press Enter → a newline is inserted, nothing saves or closes.

## Done criteria

- [ ] `npm run typecheck` exits 0
- [ ] Esc on Notes flushes before closing; the flush logic exists once
- [ ] Each of the three dialogs focuses its primary input on open
- [ ] Enter submits in Add/Edit, Settings and Move-to-folder; **not** in Notes
- [ ] Every button inside a new `<form>` has an explicit `type`
- [ ] `inert` blocks Tab into the grid while an overlay is open
- [ ] Settings refetches on open
- [ ] Nothing under `src-tauri/`, `src/store.ts` or `src/types.ts` modified
- [ ] `plans/README.md` status row for 039 updated

## STOP conditions

Stop and report back if:

- The Esc flush needs `saveNotesAction` or any store change. It does not — the
  flush is entirely local to `NotesPanel`.
- Any overlay renders **inside** the wrapper you would mark `inert`. Report the
  tree rather than restructuring it; disabling a dialog with `inert` would be a
  far worse bug than the one being fixed.
- `inert` cannot be typed without `@ts-expect-error` or a `strict` relaxation.
  Report; a hand-rolled Tab trap is explicitly out of scope and needs its own
  decision.
- Adding a `<form>` changes what the Save button does in any case beyond
  Enter-to-submit. Report rather than adjusting validation.

## Maintenance notes

- **The generalisable bug in step 1**: any panel that debounces a save and
  closes on a keyboard shortcut has this defect, because a keyboard close fires
  no blur. If a second autosaving surface is ever added, it needs the same flush
  on the same route.
- The `[notesFor]`-keyed seeding effect is deliberately *not* keyed on
  `project` — its comment explains that a save triggers a reload which would
  otherwise clobber keystrokes. Do not "fix" that dependency array; it is load
  bearing, and plan 038 reduces (but does not remove) the reload it guards
  against.
- `inert` is the whole focus-trap implementation on purpose. If a future dialog
  needs more than that, the answer is still not a hand-rolled Tab cycle — it is
  to check why `inert` is insufficient.
