# Hangar

Hangar is a Steam-library-style launcher for local dev projects. Click Run
and it pulls the repo, installs dependencies if needed, starts the dev
server, waits for the port to come up, and opens the browser. Click Stop and
the whole process tree dies, verifiably.

## Install

```
npm install
npm run build:app
```

This produces `src-tauri/target/release/bundle/macos/Hangar.app`. Drag it to
`/Applications`.

The build is **unsigned**. On first launch, macOS will refuse to open it as
coming from an "unidentified developer" — right-click the app and choose
Open once to bypass that. This is expected for a local, unsigned build, not
a bug.

## Reinstall after a change

```
npm run install:app
```

**Quit Hangar first with Cmd+Q, not Force Quit** — Cmd+Q runs the app's quit
path, which stops any dev servers it's supervising cleanly; Force Quit kills
Hangar without stopping them, orphaning those processes.

`npm run build:app` only writes to
`src-tauri/target/release/bundle/macos/Hangar.app` — `/Applications/Hangar.app`
is a separate copy, so building alone changes nothing you'll actually see.
`npm run install:app` builds and then replaces the `/Applications` copy in
one step (macOS-only; there is no Windows build to install yet).

## Releases

To cut a release, tag a commit and push the tag:

```
git tag v0.1.1 && git push origin v0.1.1
```

`.github/workflows/release.yml` does the rest: it builds the `.dmg` and
attaches it to a GitHub Release for that tag.

**The release is unsigned.** Hangar has no Apple Developer certificate, so on
first launch macOS will say *"Hangar is damaged and can't be opened."* That
is Gatekeeper rejecting an unsigned, downloaded build — not a bug, and the
app is not actually damaged. The fix: right-click the app → **Open** →
**Open**. That bypasses Gatekeeper for that app, once, per machine.

This is for **sharing** a build with someone else. For your own machine,
`npm run install:app` (above) remains the update path — there is nothing to
download, because you compile it yourself.

Releases are **macOS arm64 only**. There is no Windows build (SPEC.md's
orphan-process test has never been run on real Windows hardware, so nothing
ships there yet) and no Linux build (the CI setup it needs — a webkitgtk apt
install — is deferred; see `.github/workflows/ci.yml`'s top comment).

## Develop

```
npm run dev
```

This is `tauri dev` — it starts the Vite dev server and opens the app
through Tauri's webview, so `invoke()` calls work.

`npm run dev:web` is bare Vite with no Tauri IPC — every `invoke()` call
rejects. It is useful only for isolated frontend work, never for exercising
the app.

## Verify

```
npm run verify
```

Runs the four host gates: `cargo check`, `cargo test`, `tsc --noEmit`, and
the frontend build.

```
npm run test:acceptance
```

Runs the SPEC.md §15 process tests. The script already bakes in
`--test-threads=1`, which these tests require.

### Windows cross-check

```
PATH="/opt/homebrew/opt/llvm/bin:$PATH" \
  cargo check --manifest-path src-tauri/Cargo.toml \
  --target x86_64-pc-windows-msvc --no-default-features --all-targets
```

Compiles SPEC.md §8's `#[cfg(windows)]` code — Job Objects, `raw_arg`,
`CREATE_NO_WINDOW` — which a normal `cargo check` on macOS never looks at.
CI runs the same command on the macOS `host` job.

Both flags are load-bearing:

- `--no-default-features` turns off the `github` Cargo feature, dropping
  `reqwest` → `rustls` → `aws-lc-sys`, whose C build script needs `windows.h`
  and cannot build on macOS. Without it the check dies in that build script
  having verified nothing about Hangar. The GitHub slice is covered by
  `npm run verify` and by CI's `windows` job instead.
- `llvm-rc` on `PATH` (see Prerequisites) — otherwise `tauri-winres` panics
  with `NotAttempted("llvm-rc")` before any of `src-tauri/src/` is typechecked,
  which looks like an unrelated bug.

Sanity check that it actually got to our code, not just that it exited 0:

```
... 2>&1 | grep -cE "(Compiling|Checking) hangar"   # must be 1
```

**This proves the Windows code compiles. It does not run it.** Nothing on a Mac
can execute `TerminateJobObject`; only CI's `windows` job does that, and the
real orphan test still needs a human on Windows hardware (SPEC.md §15 test 3).

## Where your data lives

`~/Library/Application Support/com.hangar.app/` holds `projects.json` and
`settings.json`. A corrupt `projects.json` is never overwritten — it is
renamed to `projects.json.broken-<timestamp>`, and a banner in the app names
the backup.

## GitHub connection

Hangar can connect to GitHub to surface notifications for the repositories you register
(SPEC.md §18) — this is the one place the app makes a network call or stores a credential.
Connect a classic personal access token from the Inbox panel (header button). Hangar asks for
exactly two scopes:

- `notifications` — read your GitHub notifications.
- `repo` — full read/write access to your repositories' code. This is broad; grant it only if
  you understand what it lets a token do. If every repository you want to see is public,
  `public_repo` (read/write to public repos only) is enough instead.

The token is stored in your OS keychain only — never on disk, never in `projects.json`.
Disconnecting from the Inbox panel deletes it from the keychain and leaves no residue. Hangar
never asks for your GitHub password, only a token generated at
`github.com/settings/tokens`.

## Prerequisites

- Node 24+
- A Rust toolchain

Typechecking the Windows target additionally needs `llvm-rc`
(`brew install llvm`; it installs keg-only at `/opt/homebrew/opt/llvm/bin`).

## Pointers

- `SPEC.md` — the authoritative spec.
- `plans/README.md` — the implementation-plan index and status board.
