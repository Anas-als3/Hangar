# Plan 021: Make Hangar installable — a bundle script, a CI release job, and a human README

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat cc6172c..HEAD -- package.json .github/workflows/ci.yml src-tauri/tauri.conf.json`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P1
- **Effort**: S–M
- **Risk**: LOW — additive; touches no runtime code. The real risk is that the
  first-ever release build surfaces bundling problems (icons, identifier,
  signing) that have never been exercised.
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `cc6172c`, 2026-08-09

## Why this matters

SPEC.md §15 test 9 is the acceptance test the whole product is judged by:

> **The real test (human, two weeks):** you open Hangar instead of the terminal.
> If you stop doing that, the missing reason is the next feature — pulled from
> §16 or newly discovered, never from the OUT list.

That test cannot begin. Opening Hangar today requires a terminal, the repo
checked out, and a Vite dev server (`npm run tauri dev`) — the exact ritual
SPEC.md §1 exists to abolish. There is no bundle script, CI never runs
`tauri build`, and there is no README telling a human how to install or run it.

This is not one feature among several. **§16 parks ten deferred ideas, and six
of them carry promotion criteria of the form "promote if X shows up in real
use"** — Restart ("if two-week use shows frequent manual Stop→Run cycles"),
Stop All ("on real multi-project sessions"), `readyCheck: http` ("if tabs open
on blank/compiling pages"), Unix crash recovery ("if Hangar crashes actually
strand servers in practice"), port repin ("if auto-bumped ports keep
happening"), cloud-sync warnings ("if sync-lock EPERM failures show up").
Until the app is installable, none of that evidence can be gathered, so every
prioritisation decision after this one is being made blind. SPEC.md §42 and
`CLAUDE.md` both forbid building from §16 without the evidence.

Shipping an installable build is what turns the parking lot from guesswork into
a decision procedure.

## Current state

Good news first — most of the bundling groundwork already exists and needs no
change:

- `src-tauri/tauri.conf.json` is already configured: `"productName": "Hangar"`,
  `"identifier": "com.hangar.app"`, `bundle.active: true`,
  `bundle.targets: "all"`.
- `src-tauri/icons/` already contains the full set: `32x32.png`, `128x128.png`,
  `128x128@2x.png`, `icon.icns`, `icon.ico`, `icon.png`.
- `build.beforeBuildCommand` is `"npm run build"` and `frontendDist` is
  `"../dist"` — correct.

What is missing:

- `package.json` scripts (as of `cc6172c`) are `dev`, `dev:web`, `build`,
  `preview`, `tauri`, `typecheck`, `check:rust`, `test:rust`,
  `test:acceptance`, `verify`. **There is no bundle script** — `tauri build`
  is reachable only through the raw `tauri` passthrough and has, as far as the
  repo shows, never been run.
- `.github/workflows/ci.yml` has three jobs: `host` (macos-latest, runs
  `npm run verify`), `acceptance` (macos-latest, `npm run test:acceptance`),
  `windows` (windows-latest, `--all-targets` check + `cargo test`,
  `continue-on-error: true`). **None of them bundles.**
- **There is no `README.md`** at the repo root. The only human-facing docs are
  `SPEC.md` (a build spec), `CLAUDE.md` (13 lines of agent rules) and
  `plans/README.md` (an executor index). `npm run tauri dev` appears exactly
  once in the whole repo, inside `plans/001-m1-scaffold.md`.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Full verify | `npm run verify` | exit 0 |
| Release bundle (slow, 5–15 min first time) | `PATH="$HOME/.cargo/bin:$PATH" npm run tauri build` | exit 0; `.app` produced |
| YAML parse | `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"` | exit 0 |

`cargo` lives in `~/.cargo/bin`. A watchdog kills executor runs after 600 s of
no output on this repo, so **run the release build with output streaming, and
expect it to be the slowest thing in this plan.** If it is killed mid-build,
commit what you have and report — a partial run is fine, the reviewer will
finish the build.

## Scope

**In scope**:
- `package.json` (scripts only)
- `.github/workflows/ci.yml` (add a bundle job)
- `README.md` (create)
- `.gitignore` (one line — see step 4)
- `vite.config.ts` (one line — see step 4)

**Out of scope** (do NOT touch):
- Any Rust or TypeScript source. This plan ships no behaviour change.
- `src-tauri/tauri.conf.json` — already correct; changing the identifier would
  orphan the user's existing `~/Library/Application Support/com.hangar.app/`
  registry.
- Code signing, notarisation, auto-update, or any distribution mechanism. An
  unsigned local build is the goal; signing is a separate decision with cost.
- Dependencies — add nothing.

## Git workflow

- One commit per file: `Installable build: <what>`.

## Steps

### Step 1: The bundle script

Add to `package.json` scripts:

```json
    "build:app": "tauri build",
```

Leave every existing script alone — `build` must stay `tsc && vite build`,
because `tauri.conf.json`'s `beforeBuildCommand` calls it.

**Verify**: `node -e "console.log(require('./package.json').scripts['build:app'])"`
prints `tauri build`; `npm run verify` still exits 0.

### Step 2: Produce a real bundle and confirm it runs

Run `PATH="$HOME/.cargo/bin:$PATH" npm run build:app`. This is the first release
build this project has ever done; expect 5–15 minutes.

Confirm the artefacts exist:
`ls -d src-tauri/target/release/bundle/macos/Hangar.app` and
`ls src-tauri/target/release/bundle/dmg/` (a `.dmg` may or may not be produced
depending on the toolchain — the `.app` is the one that matters).

**Record in your report**: the exact artefact paths and the total build time.
If the build fails, that is the most valuable finding in this plan — report the
full error rather than working around it.

**Verify**: the `.app` bundle exists.

### Step 3: A CI job that bundles

Add a `bundle` job to `.github/workflows/ci.yml`, modelled on the existing
`host` job (checkout → `dtolnay/rust-toolchain@stable` →
`Swatinem/rust-cache@v2` with `workspaces: src-tauri` → `actions/setup-node@v4`
node 24 → `npm ci`):

- `runs-on: macos-latest`
- runs `npm run build:app`
- uploads the result with `actions/upload-artifact@v4` (path:
  `src-tauri/target/release/bundle/`, name: `hangar-macos`)
- `continue-on-error: true`, with a comment explaining why: this job exists to
  catch bundling regressions early, and a signing/toolchain hiccup on a hosted
  runner must not block the `host` job that gates real correctness. Match the
  existing `windows` job, which is non-blocking for the same class of reason.

Do NOT add a Windows or Linux bundle job in this plan — Linux needs webkitgtk
apt setup and Windows bundling is untested here. Note both as deliberate
omissions in a workflow comment.

**Verify**: the YAML parses (command in the table above), and
`python3 -c "import yaml; d=yaml.safe_load(open('.github/workflows/ci.yml')); print(sorted(d['jobs']))"`
prints all four job names.

### Step 4: Stop agent worktrees from breaking the dev server

A separate, one-line-each fix that belongs with this plan because it is the same
category of "the repo is hard to work in":

- `.gitignore` — add `.claude/`. Agent git worktrees are created under
  `.claude/worktrees/`, so today every `git status` shows `?? .claude/`.
- `vite.config.ts` — the `server.watch.ignored` array is currently
  `["**/src-tauri/**"]`. Add `"**/.claude/**"`. Those worktrees contain their
  own `node_modules` and `dist`, so Vite watches them and triggers reload
  storms; deleting a worktree mid-session has caused the dev server to cascade
  through full reloads and tsconfig cache clears.

**Verify**: `grep -n "^\.claude/" .gitignore` matches;
`grep -n "\.claude" vite.config.ts` matches; `npm run verify` exits 0.

### Step 5: The README

Create `README.md` at the repo root. Keep it short and factual — this is the
first human-facing document the project has had. Cover, in this order:

1. **What Hangar is**, in two or three sentences. Ground it in SPEC.md §1: a
   Steam-library-style launcher for local dev projects — click Run and it
   pulls, installs if needed, starts the dev server, waits for the port, and
   opens the browser; click Stop and the whole process tree dies verifiably.
2. **Install**: `npm install`, then `npm run build:app`, then drag
   `src-tauri/target/release/bundle/macos/Hangar.app` to `/Applications`.
   State plainly that the build is **unsigned**, so first launch needs
   right-click → Open once ("unidentified developer"), and that this is
   expected for a local build.
3. **Develop**: `npm run dev` (which is `tauri dev`) — and note that
   `npm run dev:web` is bare Vite with no Tauri IPC, so every `invoke()`
   rejects; it is not the way to run the app.
4. **Verify**: `npm run verify` runs the four host gates; `npm run
   test:acceptance` runs the §15 process tests and **requires
   `--test-threads=1`**, which the script already bakes in.
5. **Where your data lives**: `~/Library/Application Support/com.hangar.app/`
   — `projects.json` and `settings.json`. Mention that a corrupt
   `projects.json` is never overwritten; it is renamed to
   `projects.json.broken-<timestamp>` and a banner names the backup.
6. **Prerequisites**: Node 24+, a Rust toolchain. Note that the Windows-target
   typecheck additionally needs `llvm-rc` (`brew install llvm`, keg-only at
   `/opt/homebrew/opt/llvm/bin`).
7. **Pointers**: `SPEC.md` is the authoritative spec; `plans/README.md` is the
   implementation-plan index and status board.

Do not restate the spec. Do not add badges, screenshots, or a licence — none
exists to claim.

**Verify**: `test -f README.md`; read it back and confirm every command in it
actually exists in `package.json`.

### Step 6: Gates and commit

**Verify**: `npm run verify` exits 0; `git status --short` shows only in-scope
files.

## Test plan

No automated tests — this plan ships no behaviour. The verification is:

- the release build produces a launchable `.app` (step 2, recorded in the report)
- the workflow parses and declares four jobs (step 3)
- every command quoted in the README exists in `package.json` (step 5)

Manual check left for the maintainer, since a subagent cannot drive a GUI: drag
the `.app` to `/Applications`, launch it from Spotlight, and confirm the
existing registry loads (the identifier is unchanged, so the current IELTS Coach
project should appear).

## Done criteria

- [ ] `npm run build:app` exists and produced a real `.app` (path + build time in the report)
- [ ] `.github/workflows/ci.yml` parses and has four jobs including `bundle`
- [ ] `README.md` exists and every command it quotes is in `package.json`
- [ ] `.gitignore` contains `.claude/`; `vite.config.ts` ignores `**/.claude/**`
- [ ] `npm run verify` exits 0
- [ ] No Rust or TypeScript source file modified
- [ ] `plans/README.md` status row for 021 updated

## STOP conditions

Stop and report back if:

- `npm run build:app` fails. Report the full error. This is the first release
  build ever attempted here and a genuine failure is the plan's most valuable
  output — do not work around it, do not disable bundle targets to get past it.
- The build demands a signing identity or an Apple Developer certificate. An
  unsigned local build is the goal; configuring signing is a separate decision.
- Bundling requires changing `identifier` in `tauri.conf.json`. That would
  orphan the user's existing config directory and their registered projects.
- You are tempted to add auto-update, a release workflow, or notarisation.
  All are out of scope.

## Maintenance notes

- Once this lands, SPEC.md §15 test 9 can finally begin. That two-week trial is
  the gate on every §16 promotion — Restart and Stop All in particular are
  waiting on exactly this evidence, and both are cheap to build once earned
  (`run::stop_all` already exists at `src-tauri/src/run.rs:1487`, unexposed).
- The `bundle` CI job is `continue-on-error` deliberately. If bundling breaks,
  that is worth knowing but must not block the gates that cover correctness.
  Revisit if the app is ever distributed to anyone other than its author.
- Signing/notarisation is the obvious next question the moment this is shared
  with a second person. It is deliberately not addressed here.
