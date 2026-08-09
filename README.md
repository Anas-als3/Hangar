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

## Where your data lives

`~/Library/Application Support/com.hangar.app/` holds `projects.json` and
`settings.json`. A corrupt `projects.json` is never overwritten — it is
renamed to `projects.json.broken-<timestamp>`, and a banner in the app names
the backup.

## Prerequisites

- Node 24+
- A Rust toolchain

Typechecking the Windows target additionally needs `llvm-rc`
(`brew install llvm`; it installs keg-only at `/opt/homebrew/opt/llvm/bin`).

## Pointers

- `SPEC.md` — the authoritative spec.
- `plans/README.md` — the implementation-plan index and status board.
