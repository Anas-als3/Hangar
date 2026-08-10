# Plan 044: A tagged release that produces a downloadable `.dmg`

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `ls .github/workflows/ && grep -n "bundle:" .github/workflows/ci.yml`
> `ci.yml` must exist and contain a `bundle:` job. On a mismatch, STOP.

## Status

- **Priority**: P3 — nothing here is urgent until someone other than the
  maintainer needs a build
- **Effort**: S
- **Risk**: LOW — additive CI and docs; touches no application code
- **Depends on**: plan 021 (DONE — `npm run build:app` produces a working
  `.app` and `.dmg`)
- **Category**: dx
- **Planned at**: 2026-08-10
- **Implements**: plan 024's **tier 1** only. Read plan 024 before starting;
  tiers 2 and 3 cost money or ship an untested Windows path and are **not** in
  scope.

## Why this matters

The maintainer asked: *"how can i download the new version, there should be a
setup file that downloads the new version each time no?"*

For their own machine the answer is `npm run install:app` — there is nothing to
download, because they compile it. This plan covers the other half: producing a
**link** they can send someone.

Today the only way to get a build off this machine is to hand over the `.dmg`
from `src-tauri/target/release/bundle/dmg/`. A tagged GitHub Release turns that
into a URL, using the `bundle` job's existing recipe.

## What this deliberately does not do

- **No code signing, no notarisation.** That is plan 024 tier 2: an Apple
  Developer account at **$99/year**. Without it, macOS Gatekeeper tells anyone
  who downloads the `.dmg` that *"Hangar is damaged and can't be opened"* —
  alarming and false. The workaround is right-click → **Open** → **Open**, and
  the README must say so plainly.
- **No Windows or Linux artifacts.** `ci.yml`'s top comment records why Linux is
  deferred (webkitgtk apt setup) and its `windows` job is `continue-on-error`.
  Plan 024 is explicit: **Windows must not ship to anyone until SPEC.md §15
  test 3 has been run on real Windows hardware**, and nothing on this machine
  can do that.
- **No auto-updater.** Tauri's updater is a **plugin**, and SPEC.md §4 pins the
  plugin list to exactly three. That is a §4 amendment plus a signing key plus a
  hosted manifest, and plan 024's verdict stands: *"Not recommended for a tool
  with a handful of users."*

## The version question, which must be settled first

`package.json` and `src-tauri/tauri.conf.json` both carry `"version"` and are
edited by hand, independently. Two versions that disagree is the kind of thing
nobody notices until a bug report cites the wrong one.

**This plan makes the tag the source of truth for what is released, and adds a
check that refuses to release when the two files disagree with each other.** It
does **not** add a tool that rewrites them — that is a bigger decision and the
maintainer has not asked for it.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Workflow syntax | `python3 -c "import sys,json; import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"` | no output (if `pyyaml` is absent, skip and say so) |
| TypeScript | `npm run typecheck` | exit 0 (proves `package.json` still parses) |

**Do not run** `npm run build`, `npm run build:app`, `npm run install:app`,
`cargo` anything, `npm run verify`, or `npm run test:acceptance`. **Do not run
`gh release create`, do not push a tag, and do not trigger the workflow** — this
plan writes CI, it does not operate it.

## Scope

**In scope**:
- `.github/workflows/release.yml` — new
- `README.md` — a "Releases" section

**Out of scope** (do NOT touch):
- `.github/workflows/ci.yml`. The existing `bundle` job exists to catch
  bundling regressions on every push and **must stay fast and
  `continue-on-error`**. Plan 024 makes this a STOP condition: the release
  workflow is additive, never a modification of `bundle`.
- Anything under `src/` or `src-tauri/`. No application code changes.
- `package.json`'s version value, `tauri.conf.json`'s version value. Read them;
  do not rewrite them.
- Signing, notarisation, `APPLE_*` secrets, entitlements.
- Any new dependency, npm or GitHub Action beyond the ones `ci.yml` already uses
  (`actions/checkout`, `actions/setup-node`, `dtolnay/rust-toolchain`,
  `Swatinem/rust-cache`, `actions/upload-artifact`) plus `softprops/action-gh-release`
  or the `gh` CLI — pick one and justify it in a comment.

## Git workflow

- One commit per step: `Release: <what>`.

## Steps

### Step 1: The workflow

Create `.github/workflows/release.yml`:

- **Trigger**: `on: push: tags: ['v*']` only. Never on branch pushes — that is
  `ci.yml`'s job and duplicating it would double every build.
- **Permissions**: `contents: write` (needed to create the Release). Set it at
  the job level, not the workflow level, and add a comment saying why it is
  needed — a workflow with write permissions deserves an explanation in place.
- Steps, mirroring `ci.yml`'s `bundle` job (read it and match its structure and
  action versions exactly): checkout → Rust toolchain → `Swatinem/rust-cache`
  with `workspaces: src-tauri` → Node 24 with `cache: npm` → `npm ci` →
  `npm run build:app`.
- **A version-agreement check before the build**: read `version` from
  `package.json` and from `src-tauri/tauri.conf.json`; if they differ, **fail
  the job** with a message naming both values. If the tag is `vX.Y.Z`, also
  require it to match. Plain `node -e` or `python3` — no new dependency.
- Attach `src-tauri/target/release/bundle/dmg/*.dmg` to the Release.
- **Do not** mark it `continue-on-error`. `bundle` is informative; a release
  that half-fails must be loud.
- The Release body must contain the Gatekeeper instruction verbatim — see step 2.

**Verify**: the YAML parses (or say so if `pyyaml` is unavailable);
`grep -c "continue-on-error" .github/workflows/release.yml` → `0`.

### Step 2: README

Add a **Releases** section:

- How to cut one: `git tag v0.1.1 && git push origin v0.1.1`, and that the
  workflow does the rest.
- **The Gatekeeper paragraph, stated plainly**: a downloaded build is unsigned,
  macOS will say *"Hangar is damaged and can't be opened"*, and the fix is
  right-click → **Open** → **Open**. Say **why**: the app is not signed with an
  Apple Developer certificate. Do not soften it — someone will hit this and the
  README is where they will look.
- One line noting that **for the maintainer's own machine, `npm run install:app`
  is the update path** and a release is for sharing.
- One line that releases are **macOS arm64 only**, with the reason.

**Verify**: `grep -c "right-click" README.md` → at least 1.

### Step 3: Self-check

Report each:

- `git diff --stat` → exactly two files, one new.
- `grep -n "on:" .github/workflows/release.yml` → tags only, no `branches`.
- `grep -n "permissions" .github/workflows/release.yml` → present, job-scoped, commented.
- `git diff --name-only` → `.github/workflows/ci.yml` is **not** listed.

**Verify**: `npm run typecheck` → exit 0.

## Test plan

There is no way to test a release workflow without cutting a release, and this
plan explicitly does not do that.

Manual check for the maintainer, when they choose to try it:

- `git tag v0.1.1 && git push origin v0.1.1` → the workflow runs, a Release
  appears with a `.dmg` attached, and the body carries the right-click
  instruction.
- Deliberately set `package.json`'s version to something else and tag again →
  the job **fails** at the version check, before spending build minutes.
- Download the `.dmg` on another Mac → Gatekeeper blocks a double-click, and
  right-click → Open works.

## Done criteria

- [ ] `.github/workflows/release.yml` exists, triggers on tags only, is not
      `continue-on-error`, and fails on a version mismatch
- [ ] `ci.yml` is byte-unchanged
- [ ] README documents the tag command, the Gatekeeper step and its reason, and
      that releases are macOS arm64 only
- [ ] No `src/` or `src-tauri/` change; no signing; no updater
- [ ] `plans/README.md` status row for 044 updated

## STOP conditions

Stop and report back if:

- The workflow seems to need `APPLE_*` secrets, a certificate, or any paid
  account. It does not — that is plan 024 tier 2 and a maintainer decision.
- Adding it requires editing `ci.yml`'s `bundle` job. It must be purely
  additive; plan 024 names this as a STOP condition.
- You are tempted to add Windows or Linux artifacts. Windows must not ship until
  §15 test 3 runs on real hardware; Linux needs a webkitgtk setup `ci.yml`
  already documents as deferred.
- You are tempted to add the Tauri updater plugin. That is a §4 amendment.

## Maintenance notes

- The version check is the load-bearing part. Without it the first release
  someone else downloads can report a version that matches neither file, and
  there is no way to tell after the fact which commit it came from.
- If notarisation is ever added (plan 024 tier 2), it goes **in this workflow**,
  not in `ci.yml` — notarisation takes minutes and must never sit on the
  critical path of the per-push job that catches regressions.
- The repo is currently **private**. If it is ever made public, the Release
  page and every artifact become public with it; that is a decision to make
  deliberately, not a side effect of a tag.
