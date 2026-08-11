# Plan 035: Show the real stack — workspaces, a wider library set, and a hover that says something

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "LIBRARY_ALLOW_LIST\|fn detect_stack\|fn read_package_json" src-tauri/src/registry.rs`
> All three must exist. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. **Your reviewer runs `npm run build` and the bundle.**
> CLAUDE.md requires all three before "done"; the 600 s watchdog is why they
> are split. This resolves the contradiction the plans have carried since 023.

## Status

- **Priority**: P2 — maintainer-requested, and one of the two things they asked
  for is not delivered at all today
- **Effort**: M
- **Risk**: MED — adds filesystem reads to a path that runs on every Run, while
  plan 010 (blocking I/O under async locks) is still TODO
- **Depends on**: SPEC.md §5 amendment (ratified 2026-08-10)
- **Category**: feature
- **Planned at**: 2026-08-10, after a nine-agent design pass

## Why this matters

The maintainer, verbatim:

> the stack dont show on the auto job applier, i want the stack to show, like
> when i hover on it it should show the stack used to build it, and also in the
> ielts coach it should show more than just vite alone like the whole stack

Three separate defects, all confirmed against their real registry:

1. **Both `example-monorepo` cards show `{"libraries":[]}`.** They point at the
   repo root, whose `package.json` has **zero** dependencies and one devDep
   (`npm-run-all`). It declares `"workspaces": ["server","web"]`; everything real
   lives one level down.
2. **Example App shows `React · TypeScript`** because `LIBRARY_ALLOW_LIST` has 19
   entries and only those two match. `vitest`, `@testing-library/*`, `jsdom` are
   all present and none is on the list.
3. **Hover is a literal no-op.** The badge (`ProjectCard.tsx:261-265`) has **no
   `title` at all**. The libraries line's `title` is
   `libraries.join(" · ")` — with 2 libraries under a cap of 3, that string is
   **character-identical to the visible text**.

And the half of the original request that was never delivered: their server has
`openai` and `@anthropic-ai/sdk` in `dependencies` right now. The request plan
023 was built from said *"apis used in it if possilbe"*. Neither is on any list,
so no amount of workspace resolution surfaces them without step 3 below.

**Read SPEC.md §5's `stack` block before you start** — it was amended for this
plan and it is the authority.

## The rule that governs the whole design

> **Singular claims come from the registered folder's own `package.json`. Set
> claims union its declared workspace members.**

`framework` (the badge) and `portSuggestion` are singular identity claims about
the folder you registered — **root only, unchanged**. `libraries` is a set —
union across root + declared members.

This is not a stylistic choice. Unioning `framework` would put a **`Vite`** badge
on the card whose command is `npm run dev:server`, because `web/` declares
`vite`, the root declares nothing, and `FRAMEWORK_DETECTORS` is first-match-wins.
Not vaguer — **false**. The asymmetry must live in the type signature so it
cannot be forgotten.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --all-targets` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass (baseline 117 — **run it first and report what you observe**) |
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run verify`, `npm run build`, `npm run build:app`, or
`npm run test:acceptance`. Keep every Write/Edit under ~60 lines and commit
after each.

## Scope

**In scope**:
- `src-tauri/src/registry.rs` — the allow-list, `detect_stack`, `read_package_json`,
  two new functions, and tests
- `src/components/ProjectCard.tsx` — the two `title` attributes and the `+N` flex fix
- `src/components/AddEditDialog.tsx` — the framework in the full-list line
- `src/store.ts` — the quiet refresh only

**Out of scope** (do NOT touch):
- `sniff_port_suggestion`. It stays **root-only**. Add a one-line comment saying
  why; change nothing else. §10 step 4's "a suggestion, never silent magic".
- `has_dependency` — unchanged, so its call sites and tests do not churn.
- `commands.rs`'s `is_run_inert_change`, `stack_is_unchanged_ignoring_timestamp`,
  `merge_run_inert_fields`, `replace_preserving_app_owned_fields`. **Deliberately
  unchanged** — see step 4.
- `ProjectStack`'s shape, `PackageJsonInfo`'s shape, any §7 command. §7 is FROZEN
  and this plan adds no wire key.
- `run.rs`, the §6 state machine, §8 kill paths, §9 run sequence.
- Globs in `workspaces`, `pnpm-workspace.yaml`, turbo, Nx, Python, Docker.
- Any new dependency (a YAML crate is what kills pnpm support — §4).

## Git workflow

- One commit per step: `Stack: <what>`.

## Steps — build in this order; each slice is independently shippable

### Step 1: The hover (frontend only, no backend, ships alone)

In `src/components/ProjectCard.tsx`:

1. Add a small helper that builds one string: `framework · lib1 · lib2 · …`
   when a framework exists, otherwise just the libraries joined by ` · `.
   Single line, ` · ` separator, **no timestamp and no `\n`** (multi-line `title`
   renders differently across webviews and cannot be verified from here).
2. Give **both** the badge (`:261-265`) and the libraries line (`:274-285`) that
   same string as `title`. Both, not one: either can be the only element that
   renders — a Vite project with no allow-listed deps has a badge and no line; a
   monorepo root has a line and no badge — so each must carry the whole stack
   alone.
3. Restructure the libraries line so `+N` cannot be eaten by the ellipsis. Today
   `+N` sits **inside** the `truncate` `<p>`, so the one signal that a hover
   exists is clipped by the same ellipsis as the names:

```tsx
<p className="flex items-baseline gap-1 text-xs text-muted" title={/* the string */}>
  <span className="truncate">{project.stack.libraries.slice(0, 3).join(" · ")}</span>
  {project.stack.libraries.length > 3 && (
    <span className="shrink-0 text-muted/60">+{project.stack.libraries.length - 3}</span>
  )}
</p>
```

**No §11 amendment.** This adds no card element — `title` on elements §11 already
permits, exactly like the name, path and command lines around it. The flex
restructure is "visual treatment … the exact composition within the card", which
§11 explicitly leaves free.

**Do NOT build a custom tooltip or popover.** It would be a third card element
(§11's list is closed), §11's Motion allow-list is exhaustive and names no
tooltip, and `hover:-translate-y-0.5` on the card root makes the card a
containing block for `position: fixed` *precisely while hovered*, so a
portal-free tooltip misrenders and rides the lift.

**Verify**: `npm run typecheck` → exit 0.

### Step 2: The Edit dialog shows the framework too

`AddEditDialog.tsx:259-264` renders the full library list and has **never** shown
the framework. Once step 1 lands, the card's hover would be strictly more
complete than the destination §11 designates as where "the full list remains".

Prefix the framework, and widen the gate from `stack.libraries.length > 0` to
"framework **or** libraries", so a framework-only project is not blank.

**Verify**: `npm run typecheck` → exit 0.

### Step 3: Widen `LIBRARY_ALLOW_LIST`

Keep all 19 existing entries **in place, in their existing relative order**.
Prepend two, append seven. The head-and-tail placement is why **zero existing
test expectations change** — including `library_order_is_stable_regardless_of_dependency_map_order`,
whose `["React","Axios","Zod"]` survives because those three keep their relative
positions.

**Head — external services and APIs.** First because it is the rarest and most
identity-defining group, and it is the half of the original request that was
never delivered:

```rust
("openai",            "OpenAI"),
("@anthropic-ai/sdk", "Anthropic"),
```

Add a comment stating why this is **not** a §3 violation: §3 bans building AI
*into Hangar*. This is `contains_key` against a user project's dependency map —
no model, no network, no context. That comment is mandatory; it is the most
grep-visible tripwire in the change.

**Tail — testing and automation.** Last on purpose: the group most likely to
bloat, and the one you care least about when deciding what a project *is*, so its
names land in `+N`:

```rust
("playwright", "Playwright"), ("@playwright/test", "Playwright"),
("vitest", "Vitest"), ("jest", "Jest"),
("@testing-library/react", "Testing Library"),
("@testing-library/jest-dom", "Testing Library"),
("@testing-library/user-event", "Testing Library"),
```

`detect_stack` already dedupes by display name, so the three Testing Library keys
collapse to one entry — confirm that in your report.

**Do not add**: `@types/*`, `eslint`, `prettier`, `react-dom`,
`@vitejs/plugin-react`, `jsdom`, `npm-run-all`, `cors`, `dotenv`, `yaml`, `tsx`,
`nodemon`. Each is either implied by something already shown or ambient hygiene
that distinguishes nothing. A card reading "npm-run-all" is worse than a blank
line.

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → all pass with
**no expectation edits**. Report any test you had to change — you should have to
change none.

### Step 4: The quiet refresh — **ships in the same commit as step 3**

This is not optional and it is not cosmetic. `stack_is_unchanged_ignoring_timestamp`
compares `libraries` **by value**. `run.rs:815` rewrites the stored `stack` on
every Run; `status-changed` carries only the status; the store fetches `stack`
once at startup.

So on the **first Run after step 3 ships**, stored `libraries` ≠ the store's copy
→ `is_run_inert_change` is false → `guard_update` → `guard_mutation` → the
maintainer's next note-save or Move-to-folder on that running project is refused
with *"… is running. Stop it first."*

That is exactly the defect plan 032 was written to fix, one field over, and
`cargo check` / `tsc` / `vite build` all wave it through.

In `src/store.ts`, add a refresh that fetches projects **without** setting the
`loading` flag, and call it from `applyStatusChanged` when the incoming status is
`running`:

- **Without `loading`** because `App.tsx:218` swaps the entire grid for
  "Loading…" while that flag is true. A mid-run refresh must not blank the grid
  or the phase strip.
- **On `running`, not `starting`**: the first-phase status is emitted *before*
  the stack save in `run.rs`, so refreshing on `starting` would race it.
- Swallow errors and keep the current list. A failed refresh must never clear the
  grid.

**No §6 amendment.** Leave `commands.rs` alone entirely — both alternatives are
worse, and the design pass traced them: making `stack` fully run-inert without
adding it to `merge_run_inert_fields` silently drops the Edit-open refresh;
adding it means a notes autosave *rolls back* a freshly-detected stack.

**Verify**: `npm run typecheck` → exit 0.

### Step 5: Workspace resolution

Three functions in `src-tauri/src/registry.rs`. The signatures carry the design:

```rust
/// PURE — no `&Path`, so it cannot touch the filesystem. All workspace POLICY lives here.
fn declared_workspace_dirs(package_json: &serde_json::Value) -> Vec<String>

/// The ONLY new filesystem access. `&Path` in the signature is this module's existing
/// convention for "reads files".
fn read_workspace_members(dir: &Path, root: &serde_json::Value) -> Vec<serde_json::Value>

/// PURE, boundary preserved. The asymmetry is IN THE TYPE so it cannot be forgotten:
/// `root` decides the framework, `root` + `members` decide the libraries.
fn detect_stack(root: &serde_json::Value, members: &[serde_json::Value]) -> ProjectStack

const MAX_WORKSPACE_MEMBERS: usize = 8;
```

`detect_stack`'s library loop becomes: for each allow-list entry in order, check
whether **any** of `root` or `members` has the dependency. The outer loop stays
over `LIBRARY_ALLOW_LIST`, so output is independent of member order.

`declared_workspace_dirs` accepts `workspaces` as an array of strings, **or** an
object with a `packages` array of strings (yarn classic). **Reject** an entry when
any of these holds:

| Rule | How |
|---|---|
| empty string | `is_empty()` |
| glob or negation | contains `*`, `?`, `[`, or starts with `!` |
| `..`, `.`, absolute, leading `/` or `\`, Windows drive prefix | **`Path::components()` — reject if any component is not `Component::Normal`.** One predicate, all six cases |
| escapes into `node_modules` | first `Normal` component is `node_modules` |
| more than 8 | truncate to the first 8 in declared order |

Use `Path::components()`, **not** substring tests: `./node_modules/x` defeats a
first-component check, and `contains("..")` wrongly rejects a legitimate `v1..2`.

**There must be zero `read_dir` anywhere in this change.** "Never walks
`node_modules`" is then structural rather than a check someone can forget. Depth
is 1 — a member's own `workspaces` is never read, so there is no recursion to
bound and a symlink loop is unreachable. A missing or unparseable member manifest
is skipped silently, matching `read_package_json`'s existing "not an error"
contract.

Wire it in `read_package_json` **only**, between the root parse and the return.
The missing/unparseable-root branch becomes `detect_stack(&serde_json::Value::Null, &[])`.

**Read before the lock.** `run.rs` drops its path guard, then reads, then takes
the projects lock. Your change must not move a single file read inside a lock,
and must not add a second registry write.

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → all pass.

### Step 6: Tests

Add to `registry.rs`'s test module:

1. All six existing `detect_stack` call sites gain `, &[]`. **No expectation
   changes.** Report it if any expectation had to move.
2. `declared_workspace_dirs`: an array of strings; an object with `packages`; a
   bare string `"web"`; `[1,2]`; `[{}]`; `{}` with no `packages`; a glob
   `packages/*`; `../evil`; `/abs`; `./node_modules/x`; nine entries truncating
   to eight. Each malformed shape must yield **zero** members, not an error.
3. `read_workspace_members` against a real temp dir (use the existing `scratch()`
   helper): a root declaring two members, one of which is missing → one member
   returned, no error.
4. An end-to-end `read_package_json` test on a temp monorepo whose root has no
   deps and whose members carry `express` and `react` → framework `None`,
   libraries containing both.
5. Extend `a_missing_…` / `an_unparseable_…` so the `detect_stack(&Null, &[])`
   branch is covered directly rather than by implication.
6. `openai` + `@anthropic-ai/sdk` produce `OpenAI` and `Anthropic`, first in the
   list.

**Verify**: `cargo test` → all pass; report the new total.

### Step 7: Gates and self-check

Report each:

- `grep -rn "read_dir" src-tauri/src/registry.rs` → **no matches**.
- `grep -n "fn detect_stack" src-tauri/src/registry.rs` → the two-argument form.
- `grep -c "save_projects" src-tauri/src/run.rs` → unchanged (no second write).
- `git diff --stat` shows nothing in `commands.rs` or `run.rs`.

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass;
`npm run typecheck` → 0.

## Test plan

Steps 6's unit tests are the machine-checkable part. Manual checks for the
reviewer/maintainer:

- Run each project once, then look at the cards. `example-monorepo` (both cards)
  should go from blank to `OpenAI · Anthropic · React +5`; Example App from
  `React · TypeScript` to `React · TypeScript · Vitest +1`.
- Hover the badge **and** the libraries line — both must show the full stack.
- Open Edit on Example App — the full-list line now leads with `Vite`.
- **The one that matters**: run a project, and *while it is running*, save a note
  and do a Move to folder. Both must succeed. This is what step 4 protects.
- A project with no `package.json` still shows no badge and no line.

## Done criteria

- [ ] `cargo check --all-targets` 0; `cargo test` passes (report before/after);
      `npm run typecheck` 0
- [ ] Zero `read_dir` in the change; no globs supported
- [ ] `framework` and `portSuggestion` remain root-only
- [ ] No change to `ProjectStack`/`PackageJsonInfo` shapes, no §7 change, no new
      wire key, no new dependency
- [ ] `commands.rs` and `run.rs` untouched
- [ ] The quiet refresh ships in the same commit as the allow-list widening
- [ ] `plans/README.md` status row for 035 updated

## STOP conditions

Stop and report back if:

- You find yourself calling `read_dir`, or supporting a glob. Both are cut, and
  the glob is the actual §3 line.
- A file read would end up inside a held lock, or a second `save_projects` call
  appears. Report the constraint instead.
- `detect_stack` needs a `&Path`. It must not have one — that signature is the
  filesystem boundary plan 023 established and this plan preserves.
- `ProjectStack` seems to need a new field (per-workspace attribution, for
  instance). That needs ratified §5 **and** §7 amendments and it was deferred
  deliberately — the maintainer did not ask for it.
- Widening the allow-list forces an existing test expectation to change. It
  should not; head-and-tail placement is chosen precisely to avoid that.

## Maintenance notes

- **The generalisable lesson from step 4**: any change to what `detect_stack`
  *outputs* desynchronises the store from disk on the next Run, and the §6 guard
  compares by value. A future allow-list edit has the same hazard. The quiet
  refresh is the permanent fix; do not remove it.
- Future allow-list additions follow plan 023's own rule: **name a registered
  project that came up empty or misleading.** Not "this library is popular".
- `+N` will now appear on all three cards, where it appears on none today. Plan
  026's maintenance note feared exactly this — but its stated fear was "`+N`
  always showing **and** the visible three are noise". API-first / testing-last
  ordering makes the visible three the best three by construction. If it still
  reads badly, the escape hatch is the cap integer in `ProjectCard.tsx`, **not**
  a second "show on card" list that must be kept in sync forever.
- Known and accepted: both `example-monorepo` cards show the same union, because
  they share one `path`. `React` is web's and `Express` is server's. A superset
  of the truth beats `{"libraries":[]}`.
- Found while designing this, **not fixed, worth its own one-line plan**:
  `App.tsx:218` is a bare `{loading ? <p>Loading…</p> : …}`, so every window
  focus replaces the whole grid with "Loading…" for the duration of
  `get_projects`. That is why step 4's refresh must be quiet.
