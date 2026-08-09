# Plan 024: A route from "it builds on my machine" to "someone else can run it"

> **This is a DECISION plan, not an implementation task.** Most of what follows
> costs money, requires an Apple account, or changes what the project is for.
> The maintainer reads it, decides which tier they want, and only then does an
> executor build anything. Sections marked **BUILDABLE NOW** are the exceptions.

## Status

- **Priority**: P3 — nothing here is urgent until there is someone to share with
- **Effort**: S for tier 1, M for tier 2, L for tier 3
- **Risk**: LOW technically; the risk is spending money or effort on a tier
  above what is actually needed
- **Depends on**: plans/021 (DONE — `npm run build:app` produces a working
  `.app` and `.dmg`)
- **Category**: dx
- **Planned at**: commit `d1eb12e`, 2026-08-09

## Why this matters, and what it is not

The maintainer asked for a way to share Hangar "after we reach a satisfied
version". Today `npm run build:app` produces an **unsigned** `Hangar.app` and a
`.dmg`. That is enough for the author's own machine and nobody else's: macOS
Gatekeeper blocks unsigned apps downloaded from the internet with *"Hangar is
damaged and can't be opened"* — a message that is both alarming and wrong.

**Scope check.** SPEC.md §3's OUT list bans *"Deployment, cloud, accounts,
telemetry, database"*. That entry is about what Hangar does to **your projects**
— it will not deploy them, sync them, or hold accounts. Distributing **Hangar
itself** is a different question, and SPEC.md's own header contemplates it:
*"Renaming before public release is a human decision made outside this spec."*
Nothing in this plan makes Hangar deploy anything.

## First: is the version satisfying yet?

SPEC.md §15 test 9 is the gate the spec itself sets:

> **The real test (human, two weeks):** you open Hangar instead of the terminal.

That trial became possible only when plan 021 landed (2026-08-09). Sharing
before it runs means shipping something whose core promise is untested by daily
use — and §16's ten parked ideas are all waiting on the same evidence. **The
recommendation is to run the two weeks first**, then pick a tier below.

## The three tiers

### Tier 1 — Share with people you can talk to (BUILDABLE NOW, free)

Hand someone the `.dmg` directly (AirDrop, Slack, a GitHub Release) plus one
line of instructions: **right-click the app → Open → Open**. That bypasses
Gatekeeper for that app, once, per machine.

- **Cost**: nothing.
- **Honest downside**: it looks broken to anyone who double-clicks first, and
  "right-click to open" is exactly what malware instructions say. Fine for
  colleagues, wrong for strangers.
- **Ad-hoc signing** (`codesign --sign -`) makes the app run locally without a
  certificate but does **not** satisfy Gatekeeper for downloaded files — it does
  not solve this and should not be mistaken for a fix.

**What to build**: a GitHub Release workflow triggered on a tag, uploading the
`.dmg` built by the existing `bundle` job, plus a README section stating the
right-click step and why it is needed. That is genuinely all of tier 1.

### Tier 2 — Signed and notarised for macOS (M, ~$99/year)

The real fix for Gatekeeper. Requires enrolling in the **Apple Developer
Program** ($99/year), creating a *Developer ID Application* certificate, and
notarising each build with Apple.

Tauri supports this natively — it needs these in the build environment:
`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_PASSWORD` (an app-specific password), `APPLE_TEAM_ID`. In CI
they go in GitHub Actions secrets; the certificate is base64-encoded.

- **Cost**: $99/year, plus first-time setup friction that is real but one-off.
- **Result**: double-click works, no warnings, on any Mac.
- **Note**: notarisation is per-build and takes a few minutes; it must not be on
  the critical path of the existing `bundle` job, which exists to catch
  regressions fast.

**Do not start this** until someone outside the author's machine actually needs
it. It is the correct answer *when* that happens.

### Tier 3 — Cross-platform (L)

- **Windows**: `tauri build` produces `.msi`/`.exe`. Unsigned Windows binaries
  trigger SmartScreen; an EV code-signing certificate costs several hundred
  dollars a year. **Also: Hangar's Windows path has never run a real workload.**
  CI's `windows` job compiles and runs unit tests but is `continue-on-error`,
  and SPEC.md §15 test 3 (the orphan test) has always been deferred to "a human
  on Windows" who has never materialised. Shipping a Windows build means the
  Job Object kill path meets a real user before it has met a real dev server.
- **Linux**: needs `webkitgtk` and friends installed in CI; produces
  `.deb`/`.AppImage`. No signing requirement. The cheapest of the three to add
  and the one with no runtime risk beyond the usual.

**Recommendation**: Linux is a reasonable add whenever wanted. **Windows should
not ship to anyone until §15 test 3 has been run on a real Windows machine** —
that is the app's central guarantee and it is currently unverified there.

## Auto-update — flag, do not build

Tauri has an updater plugin. SPEC.md §4 says: *"**Tauri plugins: exactly**
`tauri-plugin-dialog`, `tauri-plugin-opener`, and `tauri-plugin-single-instance`."*
Adding the updater is a **§4 amendment**, and it also implies hosting an update
manifest and signing releases with an update key.

Not recommended for a tool with a handful of users. Revisit if Hangar is ever
distributed widely enough that manual updates are a real burden.

## Versioning — a gap to close before any release

`package.json` and `src-tauri/tauri.conf.json` both carry `"version": "0.1.0"`
and are updated independently by hand. Before the first shared build, decide
which is the source of truth and whether a tag drives both. Two versions that
disagree is the kind of thing nobody notices until a bug report cites the wrong
one.

## What to do now

1. **Run SPEC.md §15 test 9** — two weeks of using Hangar instead of the
   terminal. Everything else waits on it, including §16's whole parking lot.
2. If sharing with colleagues during that time: **tier 1**, which is a release
   workflow and a README paragraph.
3. Decide the version-source-of-truth question before any tagged build.
4. Revisit tier 2 when a real person outside this machine needs it.
5. Do not ship Windows until §15 test 3 has been run there.

## Done criteria (for the tier-1 slice only, if it is built)

- [ ] A tag-triggered GitHub Actions workflow builds and attaches the `.dmg` to a Release
- [ ] The README documents the right-click-to-open step and says plainly why it is needed
- [ ] `package.json` and `tauri.conf.json` versions agree, with a documented source of truth
- [ ] No new dependency, no new Tauri plugin, no change to `src/` or `src-tauri/src/`
- [ ] `plans/README.md` status row for 024 updated

## STOP conditions

Stop and report back if:

- Building tier 1 seems to need a certificate, an Apple ID, or any paid account.
  It does not — that is tier 2.
- Adding a release workflow requires changing the existing `bundle` job. It
  should be additive; `bundle` exists to catch regressions on every push and
  must stay fast and non-blocking.
- Anyone proposes the updater plugin without a §4 amendment.

## Maintenance notes

- Every tier boundary here is a cost boundary, not a technical one. The
  technical work at each tier is small; the decision is what costs.
- If Hangar is ever renamed before public release (SPEC.md's header explicitly
  reserves that decision), the identifier `com.hangar.app` must NOT change
  without a migration — it is the path to the user's `projects.json`, and
  changing it silently orphans every registered project.
