# Plan 006: Add update-on-run and finish the interface to spec

> **Executor instructions**: Follow this plan step by step. Run every verification command
> and confirm the expected result before moving to the next step. If anything in the "STOP
> conditions" section occurs, stop and report — do not improvise. When done, update the
> status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `plans/README.md` must show 001–005 as DONE and
> `cargo check --manifest-path src-tauri/Cargo.toml` must exit 0 before you change anything.
> `run.rs` must contain the marked seam left by plan 004 where the `updating` and
> `installing` phases insert — if it does not, read the current run sequence carefully
> before editing.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: MED (`git pull` can hang on credential prompts; a killed install can corrupt state)
- **Depends on**: plans/005-m5-crud.md
- **Category**: dx + direction
- **Planned at**: commit `e74666e`, 2026-08-05
- **Reconciled at**: commit `3a5c012`, 2026-08-09 — authored before any code existed. The
  step bodies were written against SPEC.md and remain correct; the integration facts below
  were verified against the real codebase after M1–M5 and the audit fixes (007–009, 015).

## Integration facts (verified at `3a5c012` — do not re-derive)

- **The insertion point exists and is marked.** `src-tauri/src/run.rs` line ~641 contains a
  `// PLAN 006 SEAM.` comment inside `run_project`, between the §6 Run claim and the spawn.
  Steps 1–3 go exactly there. Replace the comment with the real phases.
- **§6 already supports these statuses.** `next_status` already treats `Updating` and
  `Installing` as legal, stoppable, and crashable — no state-machine change is needed, and
  changing it is out of scope. Nothing enters them yet; this plan is what does.
- **`sha2` is NOT a direct dependency.** §9 step 3 requires SHA-256 lockfile hashing. `sha2`
  is present in `Cargo.lock` transitively (via tauri) but must be added to
  `src-tauri/Cargo.toml` to be used directly. CLAUDE.md requires a one-line justification
  comment at the dependency, in the style of the existing `libc` and `win32job` entries.
- **`PhaseStrip.tsx` is a 7-line stub.** Build it out; do not create a new component.
- **Plan 008's wire guard**: any new §7 payload must be declared in `src/types.ts` AND added
  to the sample list in `every_wire_key_the_backend_emits_appears_in_types_ts` in
  `src-tauri/src/registry.rs`, or the new shape ships untested.
- **Plan 010 (TODO) will reshape registry writes** to snapshot-then-save. Match the existing
  shape when you store the lockfile hash; do not pre-empt 010.
- **Verification** uses the plan 009 npm scripts: `npm run verify`, `npm run test:rust`,
  `npm run test:acceptance`, `npm run typecheck`. `cargo` needs
  `PATH="$HOME/.cargo/bin:$PATH"`. Baseline at `3a5c012`: verify exits 0, 85 tests pass,
  3 ignored, acceptance 3 passed.

## Why this matters

This plan completes the run sequence and the interface. Two parts carry real risk:

**`git pull` hanging.** Git will block forever on an HTTPS password prompt, an SSH
passphrase, or a Credential Manager popup. A 10-second timeout does not help if the timeout
handler only kills the `git` process — git spawns `ssh` and credential-helper children that
survive and keep the terminal handle. The non-interactive environment variables in SPEC.md §9
step 2 make authentication *fail fast* rather than prompt, and the timeout must tree-kill.

**Install state corruption.** If the lockfile hash is stored after an install that failed or
was cancelled, the next Run skips the install and starts against a broken `node_modules` —
and the user has no way to recover from the UI. The hash is stored only after a *successful*
install, and never after a user-cancelled one.

The phase strip is the app's one memorable visual element, and it must encode the real
sequence: phases that were genuinely skipped this run render dimmed rather than lit.

## Current state

Plans 001–005 produced the complete app minus update-on-run and the final UI pass: scaffold
and storage, spawn helper and log pipeline, kill paths and state machine, ready-detection and
browser hand-off, and full add/edit/remove with validation.

**Read before writing code**: SPEC.md §9 steps 2–3 (the exact pull and install semantics),
§6 (Stop during `updating`/`installing`), §11 (phase strip, uptime slot, Copy button, motion
rules, palette), §12 (the git/install edge-case rows), §16 (what stays unbuilt).

## Commands you will need

See the gate table in `plans/README.md`. All five gates apply.

## Scope

**In scope**:
- `src-tauri/src/run.rs` (the `updating` and `installing` phases at plan 004's seam)
- `src-tauri/src/process.rs` (per-canonical-path mutex; lockfile hashing)
- `src/components/PhaseStrip.tsx`, `src/components/ProjectCard.tsx`,
  `src/components/LogPanel.tsx`, and the stylesheet (final §11 pass)

**Out of scope** — every item in SPEC.md §16 is parked and must NOT be built:
Restart action, Unix crash recovery / `running.json` sidecar, structured `env` field,
`readyCheck: "http"`, Stop All, explicit port repin, drag-to-reorder, schema-version wrapper,
cloud-sync warnings, system tray. If one seems necessary, that is a STOP condition, not a
green light.

## Git workflow

One commit at the end: `Add M6 update-on-run, install phase, and final UI pass`.

## Steps

### Step 1: Implement the `updating` phase (SPEC.md §9 step 2)

Only when `updateOnRun` is true and `git rev-parse --is-inside-work-tree` succeeds. If git is
**not on PATH**: write the `system` log line `git not found — skipping update` and continue
to step 3 — a missing optional tool must not fail the run (SPEC.md §12 explicitly splits this
from the `npm`-missing row, which *does* crash).

Run `git -C <path> pull --ff-only` with **all four** non-interactive environment variables
from §9 step 2: `GIT_TERMINAL_PROMPT=0`, `GIT_ASKPASS=echo`,
`GIT_SSH_COMMAND="ssh -oBatchMode=yes"`, `GCM_INTERACTIVE=never`. Ten-second timeout; on
timeout, **tree-kill git** using plan 003's kill path, because git spawns ssh and
credential-helper children.

On any failure — conflict, no remote, offline, auth — write a warning to the log and
**continue anyway** (§9 step 2). If a pull fails mentioning `index.lock`, add a log hint
naming the file; **never delete it automatically**.

**Verify**: `cargo check` → exit 0. Manual: run against a repo with no remote → the run
continues and starts normally with a warning in the log.

### Step 2: Implement the install decision and `installing` phase (SPEC.md §9 step 3)

Hash the first lockfile found, in order: `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`
(SHA-256). Run the matching install when **any** of:

- (a) `lastLockfileHash` is unset (first run ever), **or**
- (b) the current hash differs from the stored hash, **or**
- (c) `<path>/node_modules` does not exist.

No lockfile at all → skip hashing and installing entirely, with the `system` line
`no lockfile found — skipping install`.

**Install failure (nonzero exit)**: do **not** store the hash, do **not** spawn the command,
set `crashed`, toast `Install failed (exit <n>) — see the log, then Run again.` The unchanged
hash is what makes the next Run retry the install.

**User-cancelled install** (Stop during `installing`, per plan 003's state machine): also
does not store the hash, and logs that `node_modules` may be partial.

**Verify**: `cargo test` → tests cover all three install triggers, the no-lockfile skip, and
that the hash is stored only on success.

### Step 3: Add the per-canonical-path mutex

Per SPEC.md §9 step 3. Steps 1–2 take a mutex keyed by the project's **canonicalized** path.
If another project sharing the folder is already updating or installing, wait for it, then
**re-check the lockfile hash** before proceeding — which typically skips a now-redundant
duplicate install. Two projects on one repo (`dev` and `storybook`) is a legitimate setup
that plan 005 explicitly allows, and without this they would run `git pull` and `npm install`
in the same directory simultaneously.

**Verify**: `cargo test` → a test asserts two concurrent runs on the same canonical path
serialize, and the second re-reads the hash.

### Step 4: Build the phase strip

Per SPEC.md §11. A slim strip on the card's bottom edge with segments
`Pull → Install → Start → Ready`, lighting in amber (`#F5B942`) as each real phase completes,
mapped from status: `updating` → Pull, `installing` → Install, `starting` → Start,
`running` → Ready.

Phases genuinely **skipped** this run (not a git repo, no install needed) render **dimmed,
not lit** — the strip encodes what actually happened, which is the entire point of it being
the app's signature element rather than decoration.

Respect `prefers-reduced-motion`: the fill is the only motion, alongside the card hover lift.
No gradients, no glassmorphism.

**Verify**: `npm run build` → exit 0; `npx tsc --noEmit` → exit 0.

### Step 5: Final UI pass per SPEC.md §11

- **Uptime slot**: while `running`, the time slot shows uptime (`up 12 m`) computed from the
  current run's start, refreshed at **30 s granularity or coarser** — explicitly no ticking
  seconds. Otherwise it shows last-run relative time.
- **Log panel Copy button**: copies the entire retained buffer with stream prefixes via
  `navigator.clipboard.writeText`, with an `execCommand('copy')` fallback (the async
  clipboard API is unreliable in Linux webkit2gtk builds), and a brief "Copied" confirmation.
- Confirm the rest of §11 is met: fonts applied per role (Space Grotesk titles/names, Inter
  UI, JetBrains Mono logs and ports), status colors correct including `stop-failed` using the
  crashed token, cards in `projects.json` array order with no sorting, Esc closing the
  slide-over, and error messages that say what happened and what to do next.

**Verify**: `npm run build` → exit 0; `grep -rn "#F5B942\|#101623" src/components/` → no
matches (components use tokens, not raw hex).

### Step 6: Run all gates, perform the full acceptance suite, and commit

Run SPEC.md §15 tests 1–8 on this machine and record literal observations, especially:

- **test 5**: change a dependency so the lockfile changes → next Run shows the Install phase.
  Then delete `node_modules` → next Run also shows Install.
- **test 8**: Stop during a long `npm install` → the card returns to `stopped`, the install
  child is dead, and the next Run re-runs Install (proving the hash was not stored).
- **test 3** (regression): the orphan test still passes after all of this.

## Test plan

Rust unit tests (`cargo test`): all three install triggers plus the no-lockfile skip; hash
stored only after success; hash not stored after failure or cancellation; canonical-path
mutex serialization; git-missing path continues rather than failing.

Manual acceptance tests: SPEC.md §15 tests 1–8, with observations in the report.

## Done criteria

- [ ] All five gates in `plans/README.md` pass
- [ ] `cargo test` passes with the install-decision and mutex tests
- [ ] §15 tests 3, 5, and 8 verified manually with observations in the report
- [ ] `grep -rn "#F5B942\|#101623" src/components/` → no matches
- [ ] Nothing from SPEC.md §16 was built (reviewer checks the diff)
- [ ] `plans/README.md` status row for 006 updated, and all six rows read DONE
- [ ] All four non-interactive git env vars present (`grep -n "GIT_TERMINAL_PROMPT\|GIT_ASKPASS\|GIT_SSH_COMMAND\|GCM_INTERACTIVE" src-tauri/src/`)

## STOP conditions

Stop and report back if:

- A `git pull` hangs past its timeout, or git child processes survive the timeout kill.
- Test 8 shows the install phase skipped on the run after a cancelled install — that means
  the hash was stored when it should not have been, and it leaves users stuck with a broken
  `node_modules` and no UI recovery.
- Implementing a phase seems to require anything in SPEC.md §16. It does not — report it.
- The orphan test (§15 test 3) regresses after this plan's changes.

## Maintenance notes

- After this plan, v0 is feature-complete. SPEC.md §15 test 9 is the two-week human trial;
  what it surfaces should be promoted from §16, never invented fresh, and never from the §3
  OUT list.
- The most likely first promotion from §16 is the Restart action, whose urgency is reduced
  (but not eliminated) now that `stopping` plus post-kill verification make Stop-then-Run
  race-free.
- A reviewer should scrutinize: that the hash is stored **only** after a successful install,
  that the git timeout tree-kills rather than killing only the direct child, and that the
  phase strip dims skipped phases instead of lighting them.
