# Plan 023: Detect each project's stack and show it as a badge on the card

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat d1eb12e..HEAD -- src-tauri/src/registry.rs src-tauri/src/run.rs src/types.ts src/components/ProjectCard.tsx src/components/AddEditDialog.tsx`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: LOW-MED — touches the persisted schema and a §7 return shape, both
  of which have specific rules that are spelled out below.
- **Depends on**: none. SPEC.md §5, §7 and §11 were amended 2026-08-09 to
  describe this feature; this plan implements those amendments.
- **Category**: direction (maintainer-requested)
- **Planned at**: commit `d1eb12e`, 2026-08-09

## Why this matters

Scanning a grid of project cards, you cannot tell a Next.js app from a Vite one
without opening the Edit dialog and reading the command. The maintainer asked to
see each project's stack at a glance, including the API libraries it uses.

## Scope of "detection" — read this before writing any code

**Only `package.json` dependencies are read. No source file is ever parsed.**

- SPEC.md §3 forbids Docker, Docker Compose, Spring Boot and Python detection —
  v0 is Node-ecosystem only. Reading `dependencies`/`devDependencies` from
  `package.json` does not touch that ban; walking the filesystem looking for
  `Dockerfile` or `requirements.txt` would.
- SPEC.md §1: *"Hangar orchestrates. It never replaces the IDE, the terminal,
  the browser, or git."* Parsing `.ts`/`.tsx` files to find which endpoints the
  code calls is IDE work. The maintainer was asked and chose dependency reading
  explicitly. **If you find yourself opening a source file, STOP.**

## The freshness decision — already made, do not re-derive

The stack is **persisted** on the project record and refreshed at three moments:
on Add, on Edit (whenever `read_package_json` runs for that path), and during
the install phase (which already reads the lockfile).

It is deliberately **not** derived in `get_projects`. That command already does
a filesystem stat per project (`src-tauri/src/commands.rs:69`), plan 022 made
`loadRegistry()` fire on **window focus** (`src/App.tsx:124`), and plan 010
— moving blocking I/O out from under the async state locks — is still TODO.
Adding N `package.json` reads to that path would multiply a known, unfixed
problem. Persisting trades a little staleness for not making it worse, and
`detectedAt` makes the staleness visible rather than silent.

## Current state

`src-tauri/src/registry.rs`:

- `Project` ends with `notes: Option<String>` (added by plan 020), all optional
  fields using `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- `PackageJsonInfo` is the §7 return shape:

```rust
pub struct PackageJsonInfo {
    pub scripts: BTreeMap<String, String>,
    pub package_manager: PackageManager,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_suggestion: Option<u16>,
}
```

- `read_package_json(dir: &Path) -> PackageJsonInfo` already parses
  `package.json` and already sniffs dependencies for the port suggestion
  (`next` → 3000, `vite` → 5173, `react-scripts` → 3000). **The dependency map
  is already in scope there** — this plan mostly reuses a read that happens.
- `every_wire_key_the_backend_emits_appears_in_types_ts` (the plan 008 drift
  guard) builds a fully-populated `ProjectView` from a `sample()` helper and
  asserts every emitted key is declared in `src/types.ts`. **It only checks the
  samples it is fed.**

`src/components/ProjectCard.tsx` renders the status pill and, since plan 022,
a clickable port button when running. The §11 element list is otherwise closed.

## The amended spec text you are implementing

SPEC.md §5 (`Project`):

```ts
stack?: {
  framework?: string;
  libraries: string[];
  detectedAt: string;
};
```

SPEC.md §7 (`read_package_json` return) gains
`stack: { framework?: string, libraries: string[], detectedAt: string }`.

SPEC.md §11 (Card contents) now permits *"a compact **stack badge** showing the
detected framework, e.g. `Next` or `Vite`, placed with the status pill. It is
display-only and derived — never an input, never a control."*

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | 98 pass + your new ones |
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run verify`, `npm run build`, `npm run build:app`, or
`npm run test:acceptance` — a 600 s no-output watchdog has killed executor runs
here. Keep every Write/Edit under ~60 lines and commit after each.

## Scope

**In scope**:
- `src-tauri/src/registry.rs` (the `stack` field, the detection function, tests,
  and the drift guard's `sample()`)
- `src-tauri/src/run.rs` (refresh the stack during the install phase)
- `src/types.ts` (mirror both shapes)
- `src/components/AddEditDialog.tsx` (carry the detected stack into add/update)
- `src/components/ProjectCard.tsx` (the badge)

**Out of scope** (do NOT touch):
- Any source-file parsing. Ever. See "Scope of detection" above.
- `get_projects` / `to_view` — the stack is persisted, not derived there.
- A new §7 command. `read_package_json` already exists and already reads the file.
- `process.rs`, the kill paths, the §6 state machine.
- Card contents beyond the single badge §11 now permits.
- Any new dependency.

## Git workflow

- One commit per file: `Stack: <what>`.

## Steps

### Step 1: The detection function

In `src-tauri/src/registry.rs`, add a `ProjectStack` struct
(`framework: Option<String>`, `libraries: Vec<String>`, `detected_at: String`,
`#[serde(rename_all = "camelCase")]`) and a function that builds it from the
merged `dependencies` + `devDependencies` map that `read_package_json` already
parses.

Framework detection — first match wins, so the list is ordered:
`next` → `"Next"`, `nuxt` → `"Nuxt"`, `@sveltejs/kit` → `"SvelteKit"`,
`astro` → `"Astro"`, `remix`/`@remix-run/react` → `"Remix"`,
`react-scripts` → `"CRA"`, `vite` → `"Vite"`, `@angular/core` → `"Angular"`,
otherwise `None`.

Libraries — a fixed allow-list of notable packages, including the API clients
the maintainer asked for. Suggested set (adapt if a name is wrong, do not
expand wildly): `react`, `vue`, `svelte`, `express`, `fastify`, `hono`,
`tailwindcss`, `typescript`, `prisma`/`@prisma/client`, `drizzle-orm`,
`axios`, `@trpc/client`, `graphql`, `@apollo/client`, `@supabase/supabase-js`,
`firebase`, `socket.io`, `zod`. Emit display names, deduped, in a stable order
(iterate the allow-list, not the dependency map, so output does not depend on
map ordering).

`detected_at` uses the existing `iso8601_utc` helper in `src-tauri/src/run.rs`
if it is public, otherwise format it the same way locally — do not add a date
crate (SPEC.md §4).

Unit-test: a Next project, a Vite project, one with no recognised framework, one
with an empty/absent `package.json` (must return an empty stack, not an error —
§10 step 6 allows projects with no `package.json` at all), and one asserting
library order is stable regardless of dependency-map order.

**Verify**: `cargo test` → new tests pass.

### Step 2: Wire it into `read_package_json` and the persisted record

Add `stack: ProjectStack` to `PackageJsonInfo` (non-optional — always returned,
possibly empty). Add `stack: Option<ProjectStack>` to `Project` and
`NewProject`, with the usual `#[serde(default, skip_serializing_if = ...)]`.

Mirror both in `src/types.ts`.

**Then set `stack` in the drift guard's `sample()`** in `registry.rs`, or
`every_wire_key_the_backend_emits_appears_in_types_ts` will pass while checking
nothing — `skip_serializing_if` means a `None` field emits no key at all. Its
own maintenance note warns about exactly this.

**Verify**: `cargo test` → all pass including the drift guard;
`npm run typecheck` → exit 0.

### Step 3: Populate on Add and Edit

In `src/components/AddEditDialog.tsx`, the `readPackageJson` result is already
fetched when a folder is picked. Carry its `stack` into the `NewProject` /
`Project` payload that Add and Save send.

Do not add a UI for editing the stack — §5 says it is app-owned, never
hand-edited.

**Verify**: `npm run typecheck` → exit 0.

### Step 4: Refresh during the install phase

In `src-tauri/src/run.rs`'s install phase, where the lockfile is already hashed
and the project record is already updated on success, refresh `stack` from a
fresh `read_package_json` of the project path at the same time.

Follow the existing `store_lockfile_hash` shape exactly — same lock discipline,
same single call site, same save path. Do NOT add a second write; fold the stack
into the same update if that is how the hash is stored.

If this turns out to require restructuring the install phase's save, STOP and
report — a stale badge is much better than disturbing the install path.

**Verify**: `cargo check` → exit 0; `cargo test` → 98+ pass.

### Step 5: The badge

In `src/components/ProjectCard.tsx`, render a compact badge next to the status
pill when `project.stack?.framework` is set — just the framework name (`Next`,
`Vite`), nothing else. §11 permits exactly this and nothing more: display-only,
never a control, no click handler, no tooltip listing libraries on the card.

Style with existing tokens only (`text-muted`, `bg-white/5`, `border-white/10`,
`font-mono` if it suits) — **no raw hex**, a reviewer greps for it. Keep it
visually quieter than the status pill; the status is what users scan for.

Render nothing when there is no framework — an "unknown" badge is noise.

**Verify**: `npm run typecheck` → exit 0.

### Step 6: Show the libraries where there is room

The libraries list has no home on the card (§11 permits the framework badge
only). Surface it in the **Edit dialog**, read-only, beneath the path — a quiet
line like `React · Tailwind · tRPC · Prisma`, with `detectedAt` rendered as a
relative time so staleness is visible.

**Verify**: `npm run typecheck` → exit 0.

### Step 7: Gates and commit

**Verify**: `cargo test` and `npm run typecheck` pass; `git status --short`
shows only in-scope files.

## Test plan

Rust: the detection tests from step 1, plus the existing drift guard now
covering the new key. No new frontend tests — there is no JS runner (SPEC.md §4).

Manual checks for the reviewer/maintainer:
- A Next project shows `Next`; a plain Node project with no framework shows no badge.
- Adding a project detects its stack immediately.
- Running a project whose deps changed refreshes the badge after the install phase.
- Editing a project shows its libraries and a plausible detected-at time.
- A project with no `package.json` still adds and runs, with no badge and no error.

## Done criteria

- [ ] `cargo test` passes with the new detection tests; `npm run typecheck` exits 0
- [ ] `stack` is set in the drift guard's `sample()`
- [ ] `grep -rn "\.tsx\?\"\|read_to_string" src-tauri/src/registry.rs` shows no source-file reading beyond `package.json`
- [ ] `grep -rnE "#[0-9A-Fa-f]{6}" src/components/` → no matches
- [ ] No new §7 command, no `get_projects` change, no new dependency
- [ ] `plans/README.md` status row for 023 updated

## STOP conditions

Stop and report back if:

- You need to open any file other than `package.json` to detect something.
- Refreshing the stack in the install phase requires restructuring how the
  lockfile hash is saved.
- The drift guard fails after adding the field — that means the Rust and TS
  shapes diverged; report rather than editing either side to make it pass.
- The badge cannot be placed without changing the card's element list beyond the
  one badge §11 permits.

## Maintenance notes

- The allow-list is the whole design. It will go stale as the ecosystem moves;
  that is acceptable and preferable to guessing from arbitrary dependency names.
  Add entries when a real project shows something missing.
- `detectedAt` exists so a wrong badge is explainable rather than mysterious. If
  a future change makes detection continuous, that field becomes redundant —
  remove it deliberately rather than leaving it lying.
- Deliberately not built: parsing source for actual API endpoints (§1 — IDE
  territory), and detecting non-Node ecosystems (§3 OUT).
