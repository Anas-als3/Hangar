# Plan 057: Preflight — "will this even start?"

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise. Update this plan's row in
> `plans/README.md` when done, unless a reviewer told you they maintain it.
>
> **Drift check**: `grep -n "fn needs_install" src-tauri/src/process.rs && grep -n "fn hash_lockfile" src-tauri/src/process.rs && grep -n "\*\*Ports\*\* (added" SPEC.md`
> All three must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. Your reviewer runs `npm run build` and the bundle.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED — this feature **reads `.env` files**. See the invariant below;
  it is the whole review surface.
- **Depends on**: nothing. No network, no auth, no new dependency.
- **Category**: feature
- **Planned at**: commit `f9b3fba`, 2026-08-11

## Why this one, ahead of the GitHub inbox

It is the only proposed feature with a **confirmed hit on the maintainer's own
machine before it was built**. Measured 2026-08-11:

- `auto-job-applier/.env.example` documents `ANTHROPIC_API_KEY`.
- `auto-job-applier/.env` **does not contain that key.**

If that project reads the key at runtime, it is already broken in the way that
presents as a mystery crash — which is exactly the class of failure that cost
the maintainer real time this week from an unrelated cause.

Also measured: `auto-job-applier` has **no `.nvmrc` and no `engines`**, while
`Ielts-Coach` pins `24.18.0` and `engines: ^22.22.2 || ^24.15.0 || >=26.0.0`.
The machine runs v24.18.0. So the Node check has a true-negative to prove it is
not just always-green.

## THE INVARIANT — read this before writing any code

**This feature reads `.env` files. A `.env` file is the single most sensitive
file in a developer's project.** The rule is absolute:

> **Parse key NAMES. Never retain, never serialize, never log, never render a
> VALUE.**

Concretely:

- The parser splits on the first `=` and **drops the right-hand side
  immediately** — the value never enters a variable that outlives the loop
  iteration, and never enters a struct field.
- `EnvFinding` (or whatever you name it) **has no field capable of holding a
  value.** Not `Option<String>`, not "redacted" — no field. A field that exists
  can be filled by a later refactor; a field that does not exist cannot.
- Nothing from a `.env` reaches a `system` log line, a §7 toast, or a panic.
- The report struct is `Serialize`. **Write a guard test that writes a `.env`
  containing a distinctive fake value, builds the full report, serializes it to
  JSON, and asserts that string does not appear anywhere in the output.** This
  is a stronger guard than §18's `Secret` could have — take it.

If you find yourself needing a value for any check, **STOP**. Every check in
scope is answerable from key names alone.

## Scope

**In scope**:
- `src-tauri/src/preflight.rs` — **new**. Pure functions plus the report types.
  Keep the filesystem at the edges: parsing and comparison take `&str` /
  `&[String]` so they test without a temp dir.
- `src-tauri/src/commands.rs` — one new command, `get_preflight`.
- `src-tauri/src/main.rs` — register it.
- `src/types.ts`, `src/api.ts`, `src/store.ts` — mirrors + `doctorOpen`.
- `src/components/DoctorPanel.tsx` — **new**, modelled on `PortsPanel.tsx`.
- `src/App.tsx` — header button, mount, `inert` set.
- `SPEC.md` §11 — the panel amendment (step 1).

**Out of scope** (do NOT build):
- **Any network call.** No OSV, no npm registry, no GitHub. That is plan 058.
- **Any fix, install, or write.** This feature **reports**. It does not create
  a `.env`, does not run `npm install`, does not write `.nvmrc`. A "Fix it"
  button is out of scope and must not appear.
- Any change to `run.rs`, `process.rs`'s existing functions, the §6 state
  machine, §8, or §9. **Preflight must not gate, delay, or block Run.**
- Any change to `projects.json`'s schema. The report is derived, never stored.
- Reading any file not named below.

## The checks — exactly these four

1. **Env keys** — for each of `.env.example` / `.env.sample` / `.env.template`
   that exists, the key names it declares that are absent from `.env`. If there
   is no example file, **the check does not run and reports nothing** — it must
   not invent a policy the project never had.
2. **Node version** — `.nvmrc` (and `engines.node` from `package.json` when
   present) versus the Node that would actually run. Reuse the existing
   `env_resolve.rs` PATH resolution — **do not write a second resolver**; §8's
   one-spawn-helper principle applies to resolution too. `engines.node` is a
   semver *range*; if honouring it needs a semver crate, **STOP and report** —
   an approximate range check that is wrong is worse than no check.
3. **Install needed** — call the existing `process::needs_install` and
   `process::hash_lockfile`. **Do not reimplement §9 step 3's three-way OR.**
   Same inputs, same answer, shown earlier.
4. **Path missing** — the project directory is gone. §12 already handles this
   on the card; the panel restates it so one place lists everything.

Each finding carries a stable `id`, a severity of `blocker | warning | note`,
a one-line human message, and the file it came from. Nothing else.

## Design rules

- **Lazy, never on the startup path.** Same rule §18 got: no preflight code may
  run before the grid renders. It runs when the panel opens, and on Refresh.
- **A snapshot, not a monitor.** Reads once on open, again only on Refresh,
  never polls. Copy the wording and behaviour of §11's Ports entry exactly.
- **Silent when clean.** A project with no findings shows one quiet line saying
  so. No green badges, no score, no "health 87%". A check that celebrates
  itself becomes noise the user learns to skip.
- **`Ok`, not `Err`.** A project whose directory is missing, whose `.env` is
  unreadable, or whose `package.json` is malformed produces a *finding*, not an
  error — §7 turns every `Err` into a toast, and a toast per project on open
  would be intolerable. Reserve `Err` for "the command itself could not run".
- **No blocking.** Run behaves exactly as it does today whether or not there
  are findings. Do not add a confirm, a gate, or a delay.

## Steps

### Step 1: Amend §11

Add a **Doctor** entry alongside the existing **Ports** and **Inbox** entries,
matching their shape: a slide-over opened from a quiet header button, one
section per project in `projects.json` array order (never sorted by severity —
same reason the grid is never re-sorted), a snapshot with a Refresh and a
stated read time, Esc closes on the same terms as the other slide-overs.

State explicitly in the amendment: **it carries no control that changes
anything** — no fix, no install, no link that writes. It reads and reports.

Also add the sentence that matters: **preflight never blocks Run.**

**Verify**: `grep -n "Doctor" SPEC.md` shows the new entry in §11.

### Step 2: `preflight.rs` + the invariant test

Types, the four pure check functions, and **the serialization guard test from
"THE INVARIANT" above — write that test first.**

Tests to include:
1. The serialization guard (a fake value never appears in the JSON).
2. A key declared in `.env.example` and absent from `.env` is found.
3. `.env.example` absent → the check yields nothing at all.
4. Comments (`# FOO=bar`), blank lines, `export FOO=bar`, and `FOO=` with an
   empty value all parse to the right key set.
5. A project path that does not exist yields one `blocker` finding and no panic.
6. Clean project → empty findings vector.

**Verify**: `cargo test` → report before/after counts.

### Step 3: The command + wire mirror

`get_preflight` returning the report for all projects. Register in `main.rs`,
mirror in `types.ts` / `api.ts`.

Feed the wire-drift guard a **fully-populated** sample — every `Option` as
`Some`, every vector non-empty. `skip_serializing_if` hides `None` fields, and
the guard passes vacuously on a sparse sample. This has bitten this repo before.

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass.

### Step 4: The panel

`DoctorPanel.tsx`, modelled on `PortsPanel.tsx` — read that file first and
follow its structure, its §11 tokens, and its Esc handling. Header button next
to Ports. Add `doctorOpen` to the `inert` overlay set in `App.tsx` and to the
folder band's Esc guard, exactly as `inboxOpen` was.

**Verify**: `npx tsc --noEmit` → 0.

### Step 5: Self-check

- `git diff --stat` → `run.rs` and the existing functions in `process.rs`
  **untouched**.
- `grep -rn "unsafe-inline\|dangerouslySetInnerHTML" src/components/DoctorPanel.tsx`
  → no match. Findings are text; render them as text.
- `grep -n "fn needs_install" src-tauri/src/preflight.rs` → **no match** (you
  called the existing one, you did not copy it).

## Test plan

Manual, for the reviewer:

- Open Doctor. `auto-job-applier` should report the missing
  `ANTHROPIC_API_KEY` **by name only**. Confirm no value is visible anywhere,
  including in the DOM.
- `Ielts-Coach` should report clean on the Node check (`.nvmrc` 24.18.0 vs
  v24.18.0 running).
- Point a project at a deleted directory → one blocker, no crash, no toast.
- Open Doctor while a project is running → Run/Stop unaffected, no phase strip
  change, no re-render storm.
- Esc closes Doctor and does not also close an open folder.

## Done criteria

- [ ] Three gates green; `cargo test` before/after reported
- [ ] **The serialization guard test exists and was mutation-tested**: break the
      parser so it retains a value, confirm the guard goes RED, restore. Report
      both outcomes.
- [ ] No network call anywhere in the diff
- [ ] `run.rs` untouched; `needs_install` called, not copied
- [ ] §11 amended; `plans/README.md` row updated

## STOP conditions

- Any check needs a `.env` **value**. None of the four does. If you think one
  does, you have expanded the scope — stop and report.
- `engines.node` needs a semver crate for a correct range check. Report rather
  than approximating.
- You find yourself adding a "Fix it" button, or writing any file.
- Preflight starts running before the grid renders, or on a timer.

## Maintenance notes

- **The invariant is the review surface.** In any future review of this module,
  check first that no type reachable from the report can hold a `.env` value,
  and that the serialization guard test still exists and still fails when the
  parser is broken. Everything else here is ordinary code.
- The temptation this feature will attract is "since we detected it, let's fix
  it." Resist it in this plan. A reporting tool that is trusted is worth more
  than a fixing tool that is feared, and writing to `.env` on a user's behalf
  is a category of action this app has never taken.
