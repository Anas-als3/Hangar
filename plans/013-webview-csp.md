# Plan 013: Turn on a restrictive webview Content-Security-Policy

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 91be38f..HEAD -- src-tauri/tauri.conf.json src/index.css index.html`
> On any change, compare the "Current state" excerpts against the live code;
> on a mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW-MED
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `91be38f`, 2026-08-06

## Why this matters

`src-tauri/tauri.conf.json` ships `"security": { "csp": null }` — the webview
runs with no Content-Security-Policy at all. Hangar's IPC surface is *process
execution* (`run_project` runs the stored free-text command through the
platform shell, by design), so CSP is the containment layer that decides
whether a compromised renderer script — a typosquatted npm dependency in the
React bundle, or a future HTML-injection sink — stays "script in a webview" or
becomes arbitrary local command execution. A strict CSP also blocks all
outbound renderer connections, which for an app whose spec (§3) explicitly
excludes telemetry and network use is pure upside. An audit on 2026-08-06
found no XSS sinks and no remote assets in the current frontend, which is
exactly why the policy is cheap to adopt NOW: everything is bundled and
same-origin.

## Current state

- `src-tauri/tauri.conf.json` (~lines 22-26):

```json
    "security": {
      "csp": null
    }
```

- Asset inventory (verified at `91be38f` — this is why the policy below works):
  - Fonts: `@fontsource/*` packages imported in `src/index.css`, bundled by
    Vite into `dist/assets/*.woff2` — same-origin, no CDN.
  - Styles: one Tailwind v4 stylesheet via `@tailwindcss/vite`; React inline
    `style` attributes are NOT used, but Vite/Tailwind inject `<style>` tags
    in dev, and some Tauri runtime paths use inline styles — `style-src` needs
    `'unsafe-inline'` (standard Tauri guidance; inline STYLE is a far smaller
    lever than inline script, which stays banned).
  - Scripts: the Vite bundle only. No eval, no remote scripts.
  - Network: the frontend calls Tauri IPC only. Tauri 2's IPC on
    macOS/Linux uses the `ipc://localhost` scheme; on Windows
    `http://ipc.localhost`. In dev, Vite serves on `http://localhost:1420`
    with an HMR websocket (`ws://localhost:1420`).
  - Images: none today (icons are the OS bundle, not webview assets). Include
    `img-src 'self' data:` anyway — Vite inlines small assets as data: URIs.
- Tauri 2 config supports both `app.security.csp` (applies to dev AND
  production) and `app.security.devCsp` (overrides csp in dev). Trust the
  schema at https://schema.tauri.app/config/2 (the `$schema` line in the file)
  if field names differ — keep the intent, comment the deviation (CLAUDE.md
  rule).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Dev smoke | `PATH="$HOME/.cargo/bin:$PATH" npm run tauri dev` | window opens, cards render, fonts correct |
| Release build | `PATH="$HOME/.cargo/bin:$PATH" npm run tauri build` | exits 0; `.app` bundle produced |
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml` | exit 0 |
| TypeScript / build | `npx tsc --noEmit && npm run build` | exit 0 |

## Scope

**In scope**:
- `src-tauri/tauri.conf.json` (the `app.security` object only)

**Out of scope** (do NOT touch):
- Any source file. If the app breaks under the policy, the fix is a directive
  adjustment in the config (or a STOP), never a code change in this plan.
- The capabilities file, plugin registrations, `set_settings` validation
  (worth doing, but it is a separate concern — noted for a later run).

## Git workflow

- Work on `main`. One commit: `Enable a restrictive webview CSP`

## Steps

### Step 1: Set the policies

Replace `"csp": null` with:

```json
    "security": {
      "csp": "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; connect-src ipc: http://ipc.localhost",
      "devCsp": "default-src 'none'; script-src 'self' http://localhost:1420; style-src 'self' 'unsafe-inline' http://localhost:1420; font-src 'self' http://localhost:1420; img-src 'self' data: http://localhost:1420; connect-src ipc: http://ipc.localhost http://localhost:1420 ws://localhost:1420"
    }
```

Rationale to preserve in a one-line comment... JSON has no comments — instead
record the rationale in the commit message body: script-src has no
unsafe-inline (Tauri injects its IPC bootstrap in a CSP-compatible way in v2);
connect-src lists both IPC scheme spellings because macOS/Linux use
`ipc://localhost` and Windows uses `http://ipc.localhost`.

**Verify**: `npx tsc --noEmit && npm run build` → exit 0 (config change can't
break these; this is the cheap regression floor).

### Step 2: Dev smoke under devCsp

`PATH="$HOME/.cargo/bin:$PATH" npm run tauri dev`. With the window open:

1. Cards render, Space Grotesk/Inter/JetBrains Mono fonts visibly load (the
   project name should NOT be a system-serif fallback).
2. Click Run on the seeded project, watch it reach `running`, click Stop.
   (Proves IPC + events flow under connect-src.)
3. Open the log panel (proves the store/panel path).
4. Check the webview devtools console (right-click → Inspect, or
   `--devtools`): ZERO CSP violation reports. A violation prints
   `Refused to ...` lines — each one names the directive to fix.

**Verify**: all four observations recorded in the report, console clean.

### Step 3: Release smoke under csp

`PATH="$HOME/.cargo/bin:$PATH" npm run tauri build`, then open the produced
app from `src-tauri/target/release/bundle/macos/Hangar.app`. Repeat step 2's
checks 1-3 (devtools may be unavailable in release — rely on visual checks:
fonts render, Run/Stop works, panel works; a CSP break here shows as missing
styles/fonts or dead IPC).

**Verify**: the release app renders styled, fonted, and Run/Stop works.

### Step 4: Gates + commit

**Verify**: `cargo check` exit 0 (untouched but cheap), `npx tsc --noEmit`,
`npm run build` exit 0; `git status` → only `src-tauri/tauri.conf.json`.

## Test plan

Manual smoke matrix in steps 2-3 (dev + release × fonts/IPC/panel/console).
No automated tests — CSP is enforced by the webview, not reachable from
`cargo test`.

## Done criteria

- [ ] `grep -n '"csp": null' src-tauri/tauri.conf.json` → no matches
- [ ] Dev smoke: 4/4 observations clean, recorded in report
- [ ] Release smoke: renders + Run/Stop works, recorded in report
- [ ] Only tauri.conf.json modified
- [ ] `plans/README.md` status row for 013 updated

## STOP conditions

Stop and report back if:

- The Tauri config schema rejects `devCsp` (schema drift from this plan) —
  report the accepted field names; do not ship a dev-hostile csp-only config
  that breaks HMR.
- IPC stops working under the policy after adding both `ipc:` and
  `http://ipc.localhost` to connect-src — the runtime may use another scheme
  in this Tauri patch version; capture the console violation line verbatim and
  report rather than adding broad sources.
- Any check requires `script-src 'unsafe-inline'` to pass — that defeats the
  policy's point; report the violating source instead.

## Maintenance notes

- Plan 005's dialogs and plan 006's phase strip add no remote assets if they
  follow the repo's bundling conventions — but their reviewers should re-run
  step 2.4 (console clean) as a one-liner check.
- Deferred deliberately: shape-validating `set_settings` input, and a
  `frame-src`/`worker-src` tightening pass if workers ever appear.
