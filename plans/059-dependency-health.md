# Plan 059: Dependency health — OSV.dev, folded into the Doctor panel

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise. Update this plan's row in
> `plans/README.md` when done, unless a reviewer told you they maintain it.
>
> **Drift check**: `grep -n "fn build_report" src-tauri/src/preflight.rs && grep -n "\*\*Doctor\*\* (added" SPEC.md && grep -n "get_preflight" src-tauri/src/main.rs`
> All three must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. Your reviewer runs `npm run build` and the bundle.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED — this is the **second** network call the app makes, and the
  first one that runs without the user having connected anything.
- **Depends on**: 057 (the Doctor panel and `preflight.rs`) — **merged**
- **Category**: feature
- **Planned at**: 2026-08-11, after 057 merged

## Why it goes in the Doctor panel, not a new button

The header already carries Add · Ports · Inbox · Doctor · Settings. A fifth
button for "security" would be the clutter §11 spends three paragraphs
preventing, and it would split one question — *is this project in good shape?* —
across two panels.

**A vulnerable dependency is a preflight finding.** It gets a `severity`, a
message and a source file, exactly like a missing env key. The panel already
renders that shape. The work here is a new *source* of findings, not new UI.

## What was measured before this was planned

`POST https://api.osv.dev/v1/querybatch`, run against a real lockfile on this
machine on 2026-08-11:

- **207 packages, one request, no API key, no registration, no rate limit.**
- Result: **0 advisories.** Both of the maintainer's projects are clean today.

Take that second number seriously. **The honest current output of this feature
is "nothing to report."** Its value is that this changes without anyone
noticing and the check costs one request. Do not build a security dashboard
around an empty result, do not add a score, and do not add a shield icon. §11's
"silent when clean" rule from 057 applies unchanged: one quiet line.

## The rules that cannot bend

- **Opt-in, and off by default.** This is the first network call the app makes
  without the user having connected anything. A dev tool that phones out on
  first launch, unasked, is a tool people uninstall. Add a Settings checkbox,
  default **off**, and say in the panel what gets sent.
- **What gets sent, stated exactly**: package names and versions from the
  lockfile. Nothing else — no path, no project name, no machine identifier.
  Write that sentence in the Settings label, not just in a comment.
- **Offline is a state, not an error.** `Ok`, never `Err` — §7 turns every
  `Err` into a toast, and 057 already established the panel renders its own
  states in place. Reuse that.
- **No retries.** Same rule as §18.
- **Bounded twice** — `reqwest`'s own timeout and an outer
  `tokio::time::timeout`, exactly as `github/client.rs` does. **Read that file
  and follow it**; do not invent a second HTTP shape.
- **Never on the startup path.** Runs when the Doctor panel opens, and only if
  the setting is on.
- **A cap on request size.** A large monorepo lockfile can hold thousands of
  entries. Cap the batch (OSV's own guidance is to batch, but keep one request
  bounded) and **`log()` or surface what was dropped** — a silent truncation
  reads as "you are clean" when it means "I did not look."

## Scope

**In scope**:
- `src-tauri/src/preflight.rs` — the new finding source, or a sibling module
  `src-tauri/src/osv.rs` if it grows past ~150 lines. Your call; say which and
  why.
- `src-tauri/src/registry.rs` — one `Settings` field, `checkDependencies`,
  default `false`.
- `src/types.ts`, `src/components/SettingsDialog.tsx` — the checkbox.
- `SPEC.md` §5 (the settings field) and §11 (one sentence in the Doctor entry).

**Out of scope** (do NOT build):
- **Any new §7 command.** `get_preflight` already exists and already returns
  findings. This adds a source, not an endpoint.
- **Any fix, upgrade, or `npm audit fix`.** 057's rule holds: the panel reads
  and reports. No "update it for me" button, ever.
- Any second HTTP client, any new dependency. `reqwest` is already in the tree.
- `npm audit` shelling out. §4 pins the spawn helper and §3 bans the
  package-manager UI; a network call to a documented API is narrower than
  spawning npm and parsing its output.
- Transitive dependency graphs, licence checks, OpenSSF scorecards. deps.dev
  offers all three and none of them is this plan.

## Lockfile parsing — the part that will be wrong first

`package-lock.json` v2/v3 has a `packages` object keyed by path
(`"node_modules/foo"`, `"node_modules/a/node_modules/b"`). The package **name
is the segment after the last `node_modules/`**, not the whole key, and the
root entry (key `""`) has no name and must be skipped.

- `pnpm-lock.yaml` and `yarn.lock` are **not JSON**. Parsing them needs a YAML
  or custom parser — a new dependency. **STOP and report** rather than adding
  one. Report "dependency check unavailable for pnpm/yarn projects" as a
  `note` finding; that is honest and costs nothing.
- Both of the maintainer's projects use `package-lock.json`, so npm-only is
  real coverage today, not a token effort.

Test the parser against a **fixture with a nested path**
(`node_modules/a/node_modules/b`) — that is the case a naive `split('/')[1]`
gets wrong, and it is silent when it does.

## Steps

1. **The setting.** `checkDependencies: bool`, default `false`, in `Settings`
   and the dialog. Ship this first so the network code can never run before the
   user has a way to say no. **Verify**: `cargo test`, existing settings tests
   still pass; a fresh `settings.json` has it `false`.
2. **The lockfile parser**, pure and tested — including the nested-path case
   and the `""` root entry.
3. **The OSV client**, following `github/client.rs`'s shape. One `send()`,
   double timeout, no retries. Offline → a `note` finding, never an `Err`.
4. **Findings.** A package with advisories becomes one finding carrying the
   package name, the installed version, and the advisory IDs. **Severity:
   `warning`, not `blocker`** — a CVE in a transitive dev dependency does not
   stop the project starting, and 057's `blocker` means "will not start".
5. **§5 and §11 amendments**, one sentence each.

Verify after each: `cargo check --all-targets` → 0; `cargo test` → report
before/after; `npx tsc --noEmit` → 0.

## Done criteria

- [ ] Three gates green; `cargo test` before/after reported
- [ ] `checkDependencies` defaults to **false**, proved by a test on a fresh
      settings file
- [ ] **No network call is possible with the setting off** — prove it with a
      test, not by reading the code
- [ ] The nested-`node_modules` parser case is tested
- [ ] No new §7 command; no new dependency
- [ ] `plans/README.md` row updated

## STOP conditions

- You need a YAML parser for `pnpm-lock.yaml`. Report the `note` finding
  instead.
- You find yourself adding a fix/upgrade button, or shelling out to `npm`.
- The feature would run with the setting off, or before the grid renders.
- OSV's response shape does not match what this plan assumes. **Report the real
  shape** — the measurement above was one query on one day, not a contract.

## Maintenance notes

- The thing to re-check in any future review: **is it still off by default, and
  does the Settings label still say exactly what is sent?** Both are one line
  from being wrong, and both are the difference between a useful check and a
  tool that quietly phones out.
- If this ever grows a "fix" button, that is the moment Hangar stops being a
  thing that only reads. That is a real product decision and belongs to the
  maintainer, not to a plan.
