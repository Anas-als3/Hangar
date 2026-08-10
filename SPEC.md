# SPEC.md — Hangar v1

> Working name: **Hangar** (a place where you keep your projects and launch them). The working name is **final for all build work** — use it literally everywhere (window title, Tauri identifier `com.hangar.app`, package names). Renaming before public release is a human decision made outside this spec; do not propose or substitute names.

## 1. What this is

A desktop app that works like a **Steam library for local dev projects**. A grid of project cards. Click **Run** on a card → the app pulls updates, installs dependencies if needed, starts the dev server, waits until it responds, and opens it in the user's default browser. Click **Stop** → the entire process tree is killed cleanly. Switching between projects becomes two clicks instead of IDE → terminal → `npm run dev` → Ctrl+C → repeat.

Hangar **orchestrates**. It never replaces the IDE, the terminal, or git. The browser tab IS the user's normal browser (Chrome/Edge) — there is **no embedded browser** in this app.

Amended 2026-08-10: Hangar **surfaces** a repository's GitHub activity and lets you reply to it (§18). It does not replace github.com — it has no repository browser, no diff viewer, no PR review UI, no merge button, and no destructive action of any kind. The test for anything proposed under §18 is: *does it tell me something I would otherwise have missed, or is it a worse version of a page GitHub already serves?* Build the first; link to the second.

## 2. The one user flow (everything serves this)

1. Open Hangar → see all registered projects as cards with live status.
2. Click **Run** on "IELTS Coach" → phases: Pull → Install (if needed) → Start → Ready.
3. Browser opens `http://localhost:<port>` automatically when the server responds.
4. User checks the app, comes back, clicks **Stop** → port is free, zero orphaned `node` processes.
5. Click **Run** on the next project → new browser tab opens for it.

## 3. Scope

### IN (v0)
- Project registry stored in a single `projects.json` (plus a two-key `settings.json` for the editor command and the §11 opt-in dependency check — the second key added 2026-08-11 by plan 059)
- Card grid UI with statuses: `stopped | updating | installing | starting | running | stopping | crashed | stop-failed`
- Run / Stop with **guaranteed process-tree kill** (the hardest and most important requirement); Stop works in **every** active phase, not just `running`
- Live log panel per project (stdout + stderr + Hangar's own system lines, last 500 lines, buffered in Rust)
- Ready-detection: poll the port (IPv4 **and** IPv6), only open browser when the server answers
- Optional per-project "update on run": `git pull --ff-only` + install if lockfile hash changed
- Add-project dialog: pick folder → read `package.json` scripts → user picks the script → set port
- "Open in editor" action (runs the configured editor command, default `code`)
- Windows AND macOS/Linux support (detect platform at runtime)

### OUT (do not build, do not scaffold, do not "prepare for")
- ❌ Embedded browser, devtools, network inspector
- ❌ AI agents, AI chat, AI context of any kind
- ❌ Plugin system / SDK
- ❌ Docker, Docker Compose, Spring Boot, Python detection (v0 is Node-ecosystem only; the manual command field makes anything else *possible* but we build no special support)
- ❌ Auto-discovery / drive scanning
- ❌ Deployment, telemetry, database (no SQLite — JSON files only)
- ⚠️ **Cloud and accounts: banned except for the GitHub integration named in §18** (amended 2026-08-10). This entry read "cloud, accounts" without qualification until the maintainer ruled, with the costs on the table, that Hangar should carry a GitHub inbox. The ban's reasons still hold everywhere else: an app that asks for credentials is one a developer evaluates for trust rather than simply uses, and every network call is a way for a local tool to become slow, flaky or offline-broken. §18 therefore permits **exactly one** provider, **one** credential, and **no** other network access of any kind. Anything beyond it is still ❌.
  - **Amended 2026-08-11 (plan 059), narrowly.** The clause above said "no other network access of any kind"; the §11 Doctor panel's dependency check is now a second exception, and this entry records it rather than letting the spec contradict the shipped code. It is granted on strictly narrower terms than §18's: **no account, no credential, no provider relationship** — an unauthenticated read of a public advisory database (`osv.dev`) — **off by default**, never on the startup or §9 Run path, sending package names and versions from `package-lock.json` and nothing else. It is **not** a precedent: a third network call needs its own amendment here, argued on its own merits, and "we already make two" is explicitly not an argument. Everything else in this entry stands ❌ exactly as written.
- ❌ System tray / background mode (window close = quit; §8's no-orphans logic depends on this contract)
- ❌ Silent port auto-detection from log output (pin + hint only, see §12)

If a task seems to need one of these, stop and flag it instead of building it. A **§16 parking lot** exists for good ideas that are deferred — add to it, never build from it.

## 4. Stack & platform rules

- **Tauri 2** (Rust backend does all process work)
- **React 18 + TypeScript (strict) + Tailwind CSS** frontend
- Scaffold with **create-tauri-app** (React + TypeScript + **Vite**). **Tailwind v4** via the `@tailwindcss/vite` plugin and a single `@import "tailwindcss";` entry stylesheet — no `tailwind.config.js`, no PostCSS setup. Verify `tauri.conf.json` `build.devUrl` matches Vite's dev port and `frontendDist` points at Vite's build output.
- **Tauri plugins: exactly** `tauri-plugin-dialog`, `tauri-plugin-opener`, and `tauri-plugin-single-instance`. Do **NOT** use `tauri-plugin-shell` — its `open` API is the deprecated path in Tauri 2; process spawning is done directly with `tokio::process` in Rust, and URL opening uses the opener plugin (called from Rust). Register single-instance **first** on the builder; on a second launch, focus the existing window and exit (it is desktop-only, registered on the Builder, and needs no capability entry).
- **Tauri 2 ACL**: create `src-tauri/capabilities/default.json`:
  ```json
  {
    "identifier": "default",
    "windows": ["main"],
    "permissions": ["core:default", "dialog:default", "opener:default"]
  }
  ```
  Register plugins in the builder: `.plugin(tauri_plugin_dialog::init()).plugin(tauri_plugin_opener::init())`. Cargo deps `tauri-plugin-dialog = "2"`, `tauri-plugin-opener = "2"`; npm packages `@tauri-apps/plugin-dialog` for the folder picker. Note: plugin functions called **from Rust** (e.g. opening the browser after ready-detection) bypass the ACL and need no capability entry; only webview-initiated calls do.
- **tokio** is used for its types only (`tokio::process::Command`, `tokio::io` line reading, `tokio::time`, `tokio::sync`) with `features = ["process", "io-util", "time", "sync", "macros"]`. Do **not** create your own runtime and do **not** use `#[tokio::main]` — `main` stays a plain `fn` with `tauri::Builder`; spawn all background tasks (log readers, ready-pollers, kill sequences) with `tauri::async_runtime::spawn`.
- **Managed state**: `app.manage(...)` holding `tokio::sync::Mutex<HashMap<String, ...>>` — use tokio's **async** Mutex because kill/wait sequences `.await` while state is consulted; never hold a `std::sync::Mutex` guard across an `.await`. Commands touching the map are `async` and lock with `.lock().await`; **take the child handle out of the map** before a long kill sequence so the lock is not held for 5+ seconds.
- **Storage**: set `identifier` in `tauri.conf.json` to `com.hangar.app` before M1 ends. `projects.json` path = `app.path().app_config_dir()?.join("projects.json")`; `std::fs::create_dir_all` the directory before first write. All reads/writes happen in Rust commands via `std::fs` — do **not** add `tauri-plugin-fs` or `tauri-plugin-store`. **All writes are atomic**: serialize, write to a temp file in the same directory, then rename over the original (atomic on both platforms). Same rules for `settings.json` (`{ "editorCommand": "code", "checkDependencies": false }` — the second key added 2026-08-11 by plan 059; it is read with a serde default so a `settings.json` written before it existed still loads and is never treated as corrupt).
  - Startup: file absent → write an **empty** `[]` (see §5). File present but unparseable → **NEVER overwrite**; rename it to `projects.json.broken-<timestamp>`, start with an empty registry, and show a persistent error banner naming the backup file and the parse error. Unknown JSON fields are ignored, never a fatal error.
- **Fonts** ship as npm packages `@fontsource/space-grotesk`, `@fontsource/inter`, `@fontsource/jetbrains-mono` (this is a pre-approved exception to the dependency rule) imported in the entry stylesheet so Vite bundles the woff2 files — no network requests at runtime. Fall back to `system-ui` / `ui-monospace`.
- **Windows-only Cargo dep**: `win32job` (Job Objects — see §8; justification: `taskkill /T` cannot guarantee tree kill). Everything else: no new dependency without a one-line justification comment at the import.

## 5. Data model & storage

```ts
type Status =
  | "stopped" | "updating" | "installing" | "starting"
  | "running" | "stopping" | "crashed" | "stop-failed";

interface Project {
  id: string;            // nanoid
  name: string;          // "IELTS Coach"
  path: string;          // absolute folder path
  command: string;       // "npm run dev" — free text, run through the platform shell
  port: number;          // 3000 — pinned per project, used for ready-check + browser URL
  url?: string;          // optional override; default `http://localhost:${port}`
  updateOnRun: boolean;  // default true
  readyTimeoutSec: number; // default 60
  lastLockfileHash?: string; // internal
  lastRunAt?: string;    // ISO — set when entering `starting`
  notes?: string;        // free-text scratchpad, user-owned; never parsed or acted on
  stack?: {              // detected from package.json — app-owned, never hand-edited (added 2026-08-09)
    framework?: string;      // "next" | "vite" | "react-scripts" | "astro" | ... | undefined. From the REGISTERED folder's own package.json ONLY, never a workspace member's — the badge is a claim about the folder you registered, so a monorepo root that declares no framework shows no badge rather than a false one (amended 2026-08-10)
    libraries: string[];     // notable deps incl. API clients: react, vue, express, openai, @anthropic-ai/sdk, tailwindcss, axios, trpc, prisma… Union of the registered folder's package.json and, when it declares npm `workspaces`, its declared member manifests — declared literal paths only, depth 1, capped, never a directory scan and never a glob (amended 2026-08-10)
    detectedAt: string;      // ISO — refreshed on Add, on Edit-open, on every Run, and on the install phase; staleness is visible, not hidden (corrected 2026-08-10: plan 025 made this per-Run)
  };
  folderId?: string;     // opaque, generated; the folder IS the set of projects sharing it (added 2026-08-10)
  folderName?: string;   // the folder's display name, denormalised onto every member
  openBrowserOnReady?: boolean; // default true — §9 step 6. False for a project with no page to open,
                                // e.g. an API-only server, where every Run otherwise costs a junk tab
}

// What the frontend receives (derived fields are computed by the backend, never persisted):
interface ProjectView extends Project {
  status: Status;
  pathExists: boolean;   // checked at startup, on registry change, and when Run is clicked
}
```

- `projects.json` is a bare pretty-printed array of `Project`. On a true first run, write an **empty array** — the §11 empty state is the first-run experience. The example below is documentation and a **dev fixture only**: seed it only when the `HANGAR_DEV_SEED` env var is set (used to verify milestones).

```json
[
  {
    "id": "ielts-coach",
    "name": "IELTS Coach",
    "path": "REPLACE_WITH_ABSOLUTE_PATH",
    "command": "npm run dev",
    "port": 3000,
    "updateOnRun": true,
    "readyTimeoutSec": 60
  }
]
```

- **`url` semantics**: shown only in the Edit dialog (placeholder shows the computed default). Ready-check, busy-check, and duplicate-port validation **always** use `port`, regardless of `url`. If a provided `url` contains an explicit port different from `port`, show a non-blocking validation warning: "URL port differs from the ready-check port."
- **`pathExists` = false** → card shows a warning badge, Run disabled, Edit and Remove enabled (Stop stays available if a child is somehow still alive).
- **Folder semantics** (added 2026-08-10): `folderId` is an opaque generated id, **never derived from the name** — two folders may share a name, exactly as on iOS. A folder is exactly the set of projects carrying the same `folderId`; **it has no record of its own**, so it cannot dangle, cannot be empty, and `remove_project` needs no cleanup step. `folderName` is denormalised onto every member, so no second file and no new command is required; a rename is N writes, and if members ever disagree (a rename interrupted mid-way) the **earliest member in array order supplies the displayed name**, with the next rename repairing the rest. Both fields are **run-inert** — nothing in §8 or §9 reads them (see §6). Neither is the project's directory on disk; that is `path`. Because both are optional and omitted when unset, `projects.json` stays a bare array and a user who never makes a folder gets a byte-identical file — §16's versioned wrapper is **not** triggered.

## 6. Status state machine

The single source of truth for what is legal. The backend **enforces** it (a command arriving in the wrong state returns an error — a double-clicked Run must be impossible to double-spawn).

| From | Action/event | To | Notes |
|---|---|---|---|
| `stopped`, `crashed` | Run clicked | `updating` → `installing` → `starting` per §9 | Run is rejected in every other status |
| `updating`, `installing`, `starting`, `running` | Stop clicked | `stopping` | sets a per-project **user-stop flag**; kills whichever child is active for the current phase |
| `stopping` | tree death confirmed | `stopped` | log line "Run cancelled by user" if user-stopped mid-phase |
| `stopping` | kill verification fails (§8) | `stop-failed` | Stop button stays available (retry) |
| `stop-failed` | Stop clicked | `stopping` | retry the kill |
| `starting` | port answers + grace | `running` | then open browser |
| `starting` | ready-timeout expires | kill tree (§8) → `crashed` | **the tree is killed first, then** the status changes — a timed-out server must never be left running |
| `updating`/`installing`/`starting`/`running` | child exits, user-stop flag **not** set | `crashed` | immediately — cancel any port polling; log `process exited with code <n>` |
| any | child exits, user-stop flag set (incl. quit-time kill) | `stopped` | a user Stop must never display as `crashed` |

- `lastRunAt` is set when entering `starting`.
- Remove/Edit while status ∉ {`stopped`, `crashed`} first shows a confirm ("<name> is running. Stop it first?"); confirming runs the full §8 kill and **waits for verification** before removing/saving.
  - **Exception — a change confined to the run-inert fields is not guarded** (added 2026-08-09, extended 2026-08-10). The run-inert set is exactly `notes`, `folderId`, `folderName` and `openBrowserOnReady`. This rule exists because mutating a project mid-run can break the run itself: changing `port` breaks Stop's port verification, changing `path` or `command` breaks the kill path. `notes`, `folderId` and `folderName` are never read by §8's spawn or kill paths or by §9's run sequence at all, so a change to them alone provably cannot affect a running project. `openBrowserOnReady` is read exactly once, by §9 step 6's ready hand-off — but **read only from the pre-run snapshot**: that hand-off runs on the `Project` captured by value at §9 step 0, before the run began, so a write to the stored record mid-run is structurally unobservable by the run in progress and can only ever affect the *next* Run. All four fields are edited precisely *while* a project is running — that's the shared reason they're in this set. An update that differs from the stored record in **any** other field is still guarded; the comparison must be structural (compare the records with every run-inert field normalised out), never a hand-written list of *guarded* fields, so a field added later is guarded by default until it is named in this sentence.
  - **App-owned fields are normalised out of that comparison too, and the caller can never write them** (added 2026-08-10). The app-owned set is `lastRunAt` and `lastLockfileHash`: the backend writes both without telling the frontend (§9 step 4 persists `lastRunAt` on every Run; the install phase persists `lastLockfileHash`), and the `status-changed` event carries only the status, so the caller's copy is stale from that moment until the next full registry read. Leaving them in the comparison would make the run-inert exemption **unreachable during exactly the window it exists for** — a card could not be filed into a folder, and a note could not be saved, while a project was `updating`/`installing`/`starting`. Both fields are preserved from the stored record on every write, guarded or not, so normalising them out cannot widen what a caller is able to change. This is a second, separately justified list, not a loosening of the rule above: a field belongs in it only if the backend writes it behind the frontend's back, and a field in neither list is still guarded by default.
  - **A run-inert update writes only the run-inert fields into the stored record — it must never replace the whole record from the caller's payload** (added 2026-08-10). The frontend's copy of `lastRunAt` and `stack` goes stale the moment a Run touches them (§9 step 4 persists both), and the status-changed event carries only the status, so a whole-record write would silently roll them back. Merge the named fields; leave every other field as stored.
- Exception inside the run sequence: a `git pull` failure during `updating` does **not** crash the run (§9 step 2 — warn and continue). An install failure during `installing` does (§9 step 3).

## 7. Command API & event contract (FROZEN)

Milestones implement a subset but may **not rename or reshape** anything here. All commands are `#[tauri::command]` returning `Result<T, String>` in Rust; errors surface as toasts. All payload structs derive `Serialize` with `#[serde(rename_all = "camelCase")]`.

```ts
// src/api.ts — the only file that calls invoke()
get_projects(): ProjectView[]
add_project(input: NewProject): ProjectView          // NewProject = Project minus id/lastLockfileHash/lastRunAt
update_project(project: Project): ProjectView
remove_project(id: string): void                     // rejected with a message if status ∉ {stopped, crashed}
run_project(id: string): void                        // fire-and-forget; progress arrives via events
stop_project(id: string): void
get_log_buffer(id: string): LogLine[]
clear_log_buffer(id: string): void
read_package_json(path: string): {                   // for the Add dialog
  scripts: Record<string, string>,
  packageManager: "npm" | "pnpm" | "yarn",
  portSuggestion?: number,
  stack: { framework?: string, libraries: string[], detectedAt: string }  // added 2026-08-09
}
open_in_editor(id: string): void
open_in_browser(id: string): void
// `checkDependencies` added 2026-08-11 (plan 059) — a field added to an existing payload, default
// false. An addition, never a rename or a reshape; no new command was added for the §11 dependency
// check, because `get_preflight` already returns findings and that check is a SOURCE of them.
get_settings(): { editorCommand: string, checkDependencies: boolean }
set_settings(s: { editorCommand: string, checkDependencies: boolean }): void

// Added 2026-08-10 for the §11 Ports panel. Additions to this frozen list are permitted;
// renames and reshapes are not. §9 step 1's read-only owner lookup already existed but was
// reachable only from inside a refused Run's error string, with no vehicle for asking it
// outside that moment.
get_port_status(): PortStatus[]                      // one entry per registered project, snapshot at call time
free_port(projectId: string, pid: number): void      // §9 step 1 conditions apply; rejects with a message otherwise
find_free_port(from: number, exclude: number[]): number | null  // Add/Edit dialog only — §10 step 4's "Choose for me"

interface PortStatus {
  projectId: string, port: number, busy: boolean,
  listenerCount: number,                             // > 1 → Hangar names nobody and offers nothing
  holder?: PortHolder,                               // only when listenerCount === 1 and the lookup parsed
  checkedAt: string                                  // ISO
}
interface PortHolder {
  name: string, pid: number,
  command?: string,                                  // Unix (`ps -o command=`); undefined on Windows
  startedAt?: string,                                // Unix (`ps -o lstart=`); undefined on Windows
  parentExited?: boolean,                            // Unix (`ps -o ppid=` === 1)
  sameUser?: boolean                                 // false → `free_port` is never offered
}
```

**Events** (frontend registers both listeners **once at app startup** into a global store — never inside the log-panel component):

```ts
// emitted on every transition; message carries e.g. the crash reason or kill-failure text
"status-changed": { projectId: string, status: Status, message?: string }

// batched — see §8 log pipeline
"log-lines": { projectId: string, lines: LogLine[] }

interface LogLine { stream: "stdout" | "stderr" | "system", line: string }
```

`system` is Hangar's own narration: git-pull warnings, "npm not found", exit codes, kill results. The frontend derives ALL status UI from `status-changed` plus one initial `get_projects()` call — no polling.

## 8. Process manager (Rust) — the critical part

### Environment resolution (macOS/Linux — do this first, it is the #1 real-world failure)

A GUI-launched app inherits launchd's minimal environment (`PATH ≈ /usr/bin:/bin:/usr/sbin:/sbin`), **not** the terminal's — and plain `sh -lc` never reads `~/.zshrc`, where nvm/fnm/volta users (the most common macOS Node setup) put their PATH. Without this step, every Run fails with `npm: command not found` for most macOS users.

- Once at app startup: resolve the user's login shell from `$SHELL` (fallback `/bin/zsh` on macOS, `/bin/bash` on Linux). Run `<shell> -ilc 'env'` (**interactive + login**, so both `.zprofile` and `.zshrc` load) with a 5 s timeout; parse the `KEY=VALUE` output and cache it as the **dev environment**. On failure: log a `system` line and fall back to the inherited environment. (The `fix-path-env` Rust crate implements this pattern for Tauri and is an acceptable alternative — one-line justification if used.)
- Every spawned child — dev command, git, installers, editor — gets the cached dev environment. If a tool still resolves to nothing, the error log line must show the PATH that was searched.

### Spawning — ONE shared helper for ALL children

Every child process Hangar ever spawns (dev command, `git rev-parse`, `git pull`, `npm/pnpm/yarn install`, `taskkill`, editor launch, port-owner lookup) goes through one spawn helper. Never construct a `Command` anywhere else — this is how the flags below cannot be forgotten.

- **Windows**: `Command::new("cmd")` with `.raw_arg("/C")` and `.raw_arg(&command)` (`std::os::windows::process::CommandExt`) so cmd receives the user's command byte-for-byte — normal arg handling applies MSVC-style quoting that cmd.exe does not parse and mangles commands containing quotes, `&`, `^`, or `%`. `creation_flags(CREATE_NO_WINDOW = 0x08000000)` on **every** spawn (helpers included — otherwise git/taskkill flash console windows). Never spawn `npm`/`pnpm`/`yarn`/`code` by bare name — they are `.cmd` batch shims that `Command::new` cannot execute; they must go through `cmd /C`.
  - **Job Objects (the actual kill guarantee)**: at spawn, create a Job Object via the `win32job` crate, set the extended limit `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, spawn the child, then **immediately** assign the child's handle to the job (spawn-then-assign is the accepted recipe with std/tokio `Command`; the suspended-spawn variant needs raw `CreateProcessW` and is not required). Do not grant breakaway. Keep the job handle in managed state. Because of KILL_ON_JOB_CLOSE, if Hangar itself dies **for any reason**, the kernel kills the whole job — no orphans even on a Hangar crash.
- **macOS/Linux**: `/bin/sh -c <command>` with the cached dev environment, `cwd = path`, and `.process_group(0)` so the whole tree shares one process group.
- **All platforms**: `stdin` is **null** on every child, so interactive prompts (npx "Ok to proceed?", husky hooks, credential prompts) fail fast instead of hanging forever. Pipe stdout and stderr.

### Log pipeline

- Read both streams line-by-line async. Decode with **lossy UTF-8** (never fail on invalid bytes); treat `\r` as a line break equivalent to `\n`; strip ANSI/VT escape sequences before emitting (v0 strips; stderr tinting comes from the `stream` field, not ANSI); truncate any single line beyond 4 KB with an appended `" …[truncated]"` marker.
- **Rust owns the buffer**: a per-project ring buffer of the last 500 `LogLine`s (`VecDeque` in managed state), appended before any event is emitted. Buffer lifecycle: **cleared at the start of each Run**; retained after exit/crash/stop until the next Run.
- **Events are batched**: flush accumulated lines at most every 100 ms as one `log-lines` event. If more than 2000 lines arrive in one flush window, keep the newest and emit a synthetic `system` line `"… <n> lines skipped"` — a crash-looping server must not freeze the frontend via the IPC bridge.
- The log panel, on open, calls `get_log_buffer` to backfill (subscribe first, then fetch, then merge — drop fetched lines already received live). Clear button calls `clear_log_buffer` and clears the store.

### Killing (acceptance-test level requirement)

- **Windows**: `TerminateJobObject` on the project's job (kills every descendant atomically, including grandchildren whose intermediate parent already exited — which `taskkill /T` structurally misses, because it walks live PPID chains only). Fallback **only if** job assignment failed at spawn: `taskkill /PID <pid> /T /F` (treat exit code 128 — "not found" — as success).
- **macOS/Linux**: `SIGTERM` to `-pgid` (negative process group), wait up to 5 s racing `child.wait()`, then `SIGKILL` to `-pgid`.
- **Verification — process death first, then the port**: Unix: `kill(-pgid, 0)` returns `ESRCH` (poll up to 3 s); Windows: job's active-process count is 0 (or TerminateJobObject succeeded). THEN confirm the port (both stacks, §9) no longer accepts. Port-only verification is a false proxy — leaked children that never listen (esbuild service, file watchers) would pass it. If either check fails → status `stop-failed`, never silently pretend it stopped.
- **Reaping**: the per-project exit-watcher task must `await child.wait()` — it is both the crash-detection trigger and the zombie reaper. Every kill path ends by awaiting the same wait future before declaring `stopped`. Never abandon a Child handle.
- Stop is valid in every active phase (§6): it tree-kills whichever child is active (git, installer, or dev command — all registered in managed state via the shared helper). A killed install must **not** store the new lockfile hash (so the next Run re-installs) and logs that `node_modules` may be partial.
- **On app quit with running projects** — Tauri 2 requires intercepting BOTH paths: (1) in `on_window_event`, match `WindowEvent::CloseRequested` — if anything is running, `api.prevent_close()` and start the confirm flow; (2) in the `tauri::Builder::run(|app, event| ...)` callback, match `RunEvent::ExitRequested { api, .. }` — if running projects exist and a `cleanup_done: AtomicBool` is false, `api.prevent_exit()` and start the same flow (this is the macOS Cmd+Q path a naive implementation misses). Confirm flow: never call blocking dialog APIs on the main thread — use the dialog plugin's async/callback confirm; on confirm, kill all trees (phase children included), set `cleanup_done = true`, then `app_handle.exit(0)`, which now passes through.
- **Honest scope of the guarantee**: children that create their own session (`setsid`/daemonize — Nx daemon, Turborepo daemon, watchman) intentionally escape the process group and are NOT Hangar's to kill; the guarantee covers the spawned group/job. On Unix, a `kill -9`'d Hangar cannot kill its children at death (Windows is covered by KILL_ON_JOB_CLOSE); startup recovery for that case is parked in §16.

## 9. Run sequence (exact order)

0. Guard: status must be `stopped` or `crashed` (else reject); re-check `pathExists` (false → warning state, do not run). Clear the project's log buffer.
1. Pre-check: try TCP connect to **both** `127.0.0.1:port` and `[::1]:port`. If **either** accepts → do not spawn; run a read-only owner lookup (2 s timeout) — macOS/Linux: `lsof -nP -iTCP:<port> -sTCP:LISTEN`; Windows: `netstat -ano | findstr :<port>` then `tasklist /FI "PID eq <pid>"` — and toast: "Port 3000 is in use by node (PID 4321) — is this project running elsewhere?" If the lookup fails or returns nothing, fall back to the generic message.

  **The lookup itself stays strictly read-only.** Until 2026-08-10 this step also said "no
  kill-that-process button in v0", and the reason was recorded three times in the code
  (`run.rs`, `process.rs`) and in plan 004: *the port's owner is very often the user's own
  terminal, and killing what Hangar did not spawn is exactly what §8 is careful never to
  claim.* That reasoning is still correct and the boundary it protects still stands — §8's
  guarantee covers only the trees Hangar spawned, and one authorised signal to one named
  process is not process ownership.

  **Amended 2026-08-10, after two foreign-process collisions in one day, each of which the
  pre-check caught correctly and then left the user to resolve in a terminal.** The §11 Ports
  panel may offer exactly one action against a foreign holder, **Free the port**, and only when
  every one of these holds:

  - the lookup named **exactly one** listening PID on that port;
  - that process runs as **the current user** (never root, never another account);
  - it is **not a project Hangar is currently managing** — those route to Stop, because §8 is
    the only path that may touch our own trees;
  - its **full command line was read**. If it could not be, the action is not offered at all: a
    truncated process name is not something a person can authorise a kill from.

  It signals **one PID: never a process group, never a tree, never a negative pid**. It requires
  a confirm naming the **port** and showing that process's full command line, PID, start time
  and whether its parent has exited. The PID, its start time and its ownership of the port are
  **re-verified inside the same call that sends the signal** — PIDs are reused, and any mismatch
  aborts without signalling. The first signal is **SIGTERM**; escalation to SIGKILL is a
  separate, separately confirmed action and is never automatic. Hangar re-probes afterwards and
  reports honestly, including "still held", never widens the blast radius on its own, and never
  chains a Run onto the confirm. **On Windows the action is unavailable** until command-line and
  start-time reads are verified on real hardware: `taskkill /PID <pid> /F` without a start-time
  guard is a weaker operation than this rule authorises.
2. If `updateOnRun` and the folder is a git repo (`git rev-parse --is-inside-work-tree`; git not found → `system` log "git not found — skipping update", skip to 3):
   - status `updating` → `git -C <path> pull --ff-only` with env `GIT_TERMINAL_PROMPT=0`, `GIT_ASKPASS=echo`, `GIT_SSH_COMMAND="ssh -oBatchMode=yes"`, `GCM_INTERACTIVE=never` — auth must fail fast, never prompt. 10 s timeout; on timeout kill the git **tree** (same kill path — git spawns ssh/credential-helper children). On any failure/offline: write warning to log, **continue anyway**. If a later pull fails mentioning `index.lock`, surface a log hint naming the file; never delete it automatically.
3. Install decision — hash the lockfile (`package-lock.json` | `pnpm-lock.yaml` | `yarn.lock`, first found; SHA-256). Run the matching install (`npm install` / `pnpm install` / `yarn`) when **any** of: (a) `lastLockfileHash` unset, (b) hash ≠ stored hash, (c) `<path>/node_modules` does not exist. No lockfile at all → skip hashing and installing; `system` line "no lockfile found — skipping install".
   - status `installing` → stream to the log. **Exit nonzero → do NOT store the hash, do NOT spawn; set `crashed`**, toast: "Install failed (exit <n>) — see the log, then Run again." Store the new hash only after success.
   - Same-folder coordination: steps 2–3 take a **per-canonical-path mutex**; if another project sharing the folder is updating/installing, wait, then re-check the hash (typically skipping a duplicate install).
4. status `starting` → spawn `command` via the §8 helper; set `lastRunAt`.
5. Poll **both** `127.0.0.1:port` and `[::1]:port` every 500 ms, **racing the child's exit** — if the child exits while `starting`, immediately set `crashed` and skip the rest (exit 0 → toast: "`<command>` finished (exit 0) without ever answering on port <port> — did you pick a script that starts a server (e.g. dev), not build?"; nonzero → toast with the exit code). The timeout budget is counted in **completed poll attempts** (`readyTimeoutSec × 2` attempts), not wall-clock — a poll gap over 5 s (system slept) does not count against the budget.
6. Any accepted connection on either stack = ready. Wait 300 ms grace, then: status `running` → open `url` in the default browser via the opener plugin (from Rust) — **unless the project sets `openBrowserOnReady: false`** (amended 2026-08-10). The status transition is unconditional; only the browser hand-off is skipped, and the run log still records readiness. Two cases motivated this, both observed: a project with no page to serve (an API-only server) gets a "Cannot GET /" tab on **every** Run since it was added, and a Stop→Run of a live-reloading dev server opens a duplicate tab the already-open one had reconnected on its own. Absent or `true`, the browser opens exactly as before — this default may not change, because opening the browser is §2's payoff and a silent opt-out would be worse than the junk tab.
7. If the budget expires: **kill the spawned tree via the §8 Stop path, wait for confirmed death, then** set `crashed`. Toast: "Server didn't answer on port <port> within <readyTimeoutSec> s, so it was stopped. If it just needs longer (e.g. a first cold compile), raise Ready timeout in Edit. Check the log — did it start on another port? Pin it in Edit."

## 10. Add / Edit / Remove flow

1. Native folder picker (dialog plugin).
2. Read `package.json`. List `scripts` as selectable options, pre-select `dev` if present, else `start`.
3. Command becomes `npm run <script>` — or `pnpm run` / `yarn` if that lockfile exists. The command field stays editable free text. (Hint text under the field: "Env vars work inline — `PORT=3001 npm run dev`, or on Windows `set PORT=3001 && npm run dev` — if the framework ignores the pinned port.")
4. Port field, prefilled by dependency sniffing: `next` → 3000, `vite` → 5173, `react-scripts` → 3000, otherwise empty and required. Beside it, a **Choose for me** control picks the first port that no registered project pins and that nothing is currently listening on, and — **in the same press** — writes the matching port token into the command field (amended 2026-08-10). Both halves are required together: Hangar's `port` is a *prediction* of what the child will bind, never an instruction to it, so pinning an arbitrary port without telling the project produces a prediction that fails `readyTimeoutSec` later and then kills a healthy server. When the framework is unknown the control offers both forms — `--port` and `PORT=` — and names what each suits. Both edits land in visible, editable fields and nothing is written until Save: this is still a *suggestion*, never silent magic.
   - **The prefill itself must stay a framework prediction, not an availability scan.** Bumping the suggestion to 5174 because 5173 happens to be busy would make the very first Run of a Vite project fail. Availability only ever enters through the explicit control above.
5. **Open browser when ready** checkbox, default **on** (§5 `openBrowserOnReady`, §9 step 6). Turn it off for a project with no page to serve.
6. Validate: no two projects may register the same **port** — show which project owns it. Two projects **may share a path** (e.g. `dev` and `storybook` from one repo on different ports) — allowed, no error; §9 step 3's per-path mutex handles the overlap.
7. No `package.json`? Allow manual command + port entry (this is how a Spring Boot project could be added by hand later).
8. Remove/Edit on a non-stopped project: confirm-and-stop first (§6). "Open in editor" runs `<editorCommand> <path>` through the §8 helper; on failure, toast: "Couldn't run 'code' — is it on your PATH? Change the editor command in Settings." Never fail silently.

## 11. UI direction

Steam-library energy, but its own identity — a launch bay for code. Dark, dense, calm.

- **Palette:** background `#0C0D11`, cards `#16181E`, text `#E9EAEE`, muted `#8A8F9C` — a neutral graphite base. Single accent: violet `#8B7BF7` (Run button, active phase). Status colors are functional only and do **not** follow the accent: running `#34D399`, starting/updating pulse in the accent, crashed & stop-failed `#F87171`, stopped slate. (Amended 2026-08-09 from the original blue-grey/amber palette; the *structure* — dark base, one raised surface, one accent, functional status colors — is the part that is load-bearing, not the specific hues.)
- **Type:** Space Grotesk for the app title and project names; Inter for UI; JetBrains Mono for logs and ports. Bundled via Fontsource (§4) — no CDN.
- **Card contents:** project name, status pill with port (`:3000` in mono), time slot — **state-dependent: while running it shows uptime ("up 12 m", refreshed at 30 s granularity or coarser — no ticking seconds); otherwise last-run relative time** — primary Run/Stop button, overflow menu: Open in browser · Open in editor · Show logs · Notes · **Move to folder…** · Edit · Remove (extended 2026-08-10). "Move to folder…" opens a small dialog listing the existing folders, a "New folder…" row, and a "Not in a folder" row; it is the required non-drag route both into and out of a folder, because §11 permits no keyboard shortcut that could substitute for the drag gesture. A `crashed` card's primary button is **Run** (retry). While `stopping`, the button shows a spinner and is disabled. Cards render in `projects.json` array order; new projects append; Remove preserves the order of the rest — no automatic re-sorting, ever, and **grouping never rewrites the array** (extended 2026-08-10). The grid is one walk of `projects` in array order: a project carrying a `folderId` is not drawn as its own tile, and the first time the walk reaches any member of a folder, that folder's tile is drawn in that position. A folder therefore occupies the array position of its **earliest member**, and every non-member keeps its position relative to every other; delete every folder and the grid returns, card for card, to the order it had before. A folder's position is *derived*, so moving or removing its earliest member relocates its tile — when that happens the **folder tile**, not the departing project, is scrolled into view, and the folder stays open. `grid-auto-flow: dense` is forbidden: backfilling the hole left by an expanded folder is automatic re-sorting under another name. An active search dissolves folders for the duration of the query, and every visible project renders as a plain card in array order. Two further elements are permitted, both display-only and derived — never inputs, and never controls except for the single reveal named below (added 2026-08-09, extended 2026-08-10): a compact **stack badge** showing the detected framework, e.g. `Next` or `Vite`, placed with the status pill; and a single quiet **libraries line** listing the notable detected dependencies, e.g. `React · Tailwind · tRPC`. The libraries line is **capped** — at most the first few fit a 14 rem card, and the rest are indicated by a count (`+3`). It must never wrap to a second line or push the primary Run/Stop button out of view; a dense grid that has to be scrolled to find the button has lost the plot. **The count is the one exception to "never controls" (amended 2026-08-10).** When the cap hides entries, `+N` is a button that reveals the full detected stack — framework, libraries and detected external services — in a read-only panel anchored to the card. This is the **only** control either element may ever carry, and it does exactly one thing: show what is already stored in `stack`. The panel lists detected names and a relative `detectedAt` and nothing else — no link, no per-entry action, no editing, no §7 command, nothing that reads or writes `projects.json`. **It must never cover the primary Run/Stop button**: the same sentence that forbids the line pushing that button out of view forbids the panel occluding it, and on a `stop-failed` card that button is the only route out of the state. It closes on Esc, on a click outside, and on a second press of `+N`; it renders instantly with no transition, so the Motion allow-list is untouched. `+N` renders only when the cap actually hides something, so a card whose stack fits gains no control at all. The Edit dialog keeps its full-list line but is no longer the sole destination: **reaching a card's own detected stack must never require an action §6 guards**, and Edit is confirm-and-stop on any project that is not `stopped` or `crashed`. Nothing here makes the stack badge, port pill, path line, command line or time slot into controls. A `crashed` or `stop-failed` card may carry **one additional element** (added 2026-08-10): a single muted line holding the `status-changed` message for that transition, truncated to one line with the full text in a `title`, opening the log panel on click, and cleared when the project next runs. It must be sourced from that event's `message`, **never** from the last line of the log buffer — the crash reason (e.g. "Install failed (exit 1) — see the log, then Run again.") never enters the buffer at all, so a last-line heuristic would print an unrelated earlier warning under a red pill as though it were the cause. Nothing similar is permitted on a healthy card. Otherwise the elements above and their order are fixed; their visual treatment (spacing, hierarchy, weight, borders, the exact composition within the card) is not, provided it uses the §11 palette tokens and type scale and keeps the card readable at a glance in a dense grid.
- **Signature element:** **every card carries** a slim **phase strip** along its bottom edge — drawn entirely **unlit** while the project is `stopped`, so a card's silhouette never changes, and lighting as each real phase completes once Run is clicked (amended 2026-08-10). Two reasons, both measured: the app's one memorable element rendered on **zero** cards in the state the app opens into; and because grid rows stretch, a 200 px stopped card next to a 240 px running one meant **pressing Run grew every other card in that row by 40 px**, pushing their Run buttons down. A fixed silhouette is the point. The unlit strip reuses the existing dim treatment exactly — no new colour, no new class, and no motion at rest. Everything below still applies once a run begins — labeled segments `Pull → Install → Start → Ready` that light up in the accent colour as each real phase completes (mapping: updating / installing / starting / running). Phases skipped this run (not a git repo, no install needed) render dimmed, not lit. This is the one memorable element; it encodes the actual sequence, not decoration. Keep everything else quiet.
- **Folders** (added 2026-08-10): a folder is a second kind of grid tile, not a card. It occupies one cell of the same track and shows exactly four things: the folder name (Space Grotesk, same size and weight as a project name, preceded by a `›`/`⌄` disclosure glyph, truncated with a `title`); a member count; a row of one small dot per member **in array order**, coloured by that member's live status (pulsing in the accent for transitional statuses, drawn as a hollow ring — while keeping its status colour — when that member's folder is missing, capped at eight then `+n`); and an aggregate line. The aggregate line is **counts, never a status**: the fragments `n stop-failed`, `n crashed`, `n running`, `n in progress`, joined by `·` in that fixed order so truncation can only drop the harmless end; when every member is `stopped` it shows the most recently run member's last-run relative time instead, matching the card's time-slot rule. A folder tile has **no status pill, no port, no stack badge, no libraries line, no phase strip and no Run/Stop button** — §6's status vocabulary belongs to projects, and a folder has no state machine. Folders are marked by **shape, not colour**: a brighter hairline border, two faint stacked edges above the top edge, the disclosure glyph, and the absence of a Run button. No new palette entry, no icon set, no emoji. A folder's own menu is exactly **Rename · Ungroup**; the word "Remove" is reserved for the destructive project action and must not appear on a folder. Rename edits the name in place (Enter commits, Esc cancels, blur commits, an empty commit reverts) — no dialog. Ungroup dissolves the folder and keeps every project. A folder whose last member leaves simply ceases to exist; a folder is never auto-dissolved at one member, because that would be a write the user did not ask for.
- **Opening a folder** (added 2026-08-10): inline, in the grid — never a slide-over, never a modal. Opening a tile expands a full-width band (`grid-column: 1 / -1`) immediately after it, holding the member cards in a nested grid on the same track with **no horizontal padding**, so a card is identical inside a folder and outside it. Overlays are for per-project detail (logs, notes); a folder is a region of the grid, and dimming the grid behind a backdrop would hide exactly the live status this app exists to show. **Any number of folders may be open at once**, and folders always start closed on launch: open/closed is ephemeral view state, never written to `projects.json` and never reset by a registry reload. **A folder auto-expands and cannot be collapsed while any member is `stop-failed`** — that is the one status whose required action, the retry Stop button, exists only on the card. The band leaves a hole in the folder's row: accepted, because filling it would reorder the grid.
- **Logs:** slide-over panel, mono font, autoscroll with pause-on-scroll-up, **Copy button** (copies the entire retained buffer with stream prefixes — `navigator.clipboard.writeText` with an `execCommand('copy')` fallback; brief "Copied" confirmation), Clear button, stderr lines tinted, `system` lines muted. **Esc closes the slide-over.** Esc also closes an open folder, but only while focus is inside that folder's band, so it can never fire alongside the card menu's own Esc or the search box's clear-on-Escape (added 2026-08-10). Esc closes the ports and inbox slide-overs on the same terms, and the folder band must yield to each exactly as it already yields to the log and notes panels — one keypress may never fire two unrelated state changes. These are the only keyboard shortcuts in v0.
- **Motion:** restrained and functional — motion exists to explain a state change, never to decorate. Allowed:
  - the phase-strip fill (the signature element — it stays the most expressive motion in the app, and nothing else may compete with it);
  - a subtle card hover lift;
  - enter/exit transitions on the surfaces that appear over the grid: the Add/Edit dialog, the Settings dialog, the log slide-over (which §11 already calls a *slide*-over), and toasts — fade and/or a short translate, ≤200 ms, ease-out;
  - colour/opacity transitions on status pills and phase segments when a status actually changes, ≤200 ms;
  - card enter/exit when a project is added or removed;
  - the ports slide-over's enter/exit, on the same terms as the log slide-over (added 2026-08-10);
  - the inbox slide-over's enter/exit, on the same terms (added 2026-08-10, §18);
  - drag-to-group feedback and folder expansion (added 2026-08-10): the dragged card drops to ~40 % opacity and a valid drop target (a card or a folder tile) shows a 2 px accent ring — **applied instantly, no transition, opacity and colour only**, no scale, no lift, no spring. The ghost that follows the pointer is direct manipulation, not animation: one detached node written imperatively outside React, one style write per pointer event, no rAF loop and no library. Under `prefers-reduced-motion` the dwell delay is unchanged and only the animated dwell ring is dropped, so the visual can never lead the timer. Expanding or collapsing a folder is instant — there is no height animation — but the member cards mounting into an opened band do play the existing card-enter fade: a card appearing is a card appearing.

  Everything else stays still. Still banned: gradients, glassmorphism, confetti, parallax, scroll-linked effects, looping/idle animation (the `stopping` spinner and the accent pulse on transitional statuses are the only loops), and any motion that delays interaction — a control must be usable on the frame it appears.

  Implement with **CSS transitions**, not JS animation loops or animation libraries. This is a performance requirement, not a style preference: the store notifies every subscriber on every log flush, so cards re-render frequently; CSS transitions are unaffected by re-render, JS-driven animation is not. No new dependency for motion (§4).

  `prefers-reduced-motion` must disable all of the above — the existing global rule in `src/index.css` already does this; keep it working.
- **Inbox** (added 2026-08-10, §18): a slide-over like the log panel, opened from a quiet **Inbox** button in the header, holding two panes — a list of GitHub notifications for the repositories the registry's projects point at, and a single thread with a reply box. **The unit is the repository, never the project**: two cards sharing one repo root produce one section, and every total is summed over distinct repositories. It shows its own disconnected, offline, rate-limited and empty states in place — none of them is a toast, and none is an error. A project that is not on GitHub, has no remote, or cannot be seen with the current token is simply **absent**, with no toast, no banner and no `system` log line: that buffer is Run narration and GitHub noise in it would make a GitHub failure look like a Run event. The header button may carry an unread count, read from the local cache only — it must never make a network call, a keychain call, or run before the grid does.
- **Resume last session** (added 2026-08-10): a single quiet line above the grid naming the projects that were running together when Hangar last quit, and one button that starts them. The set is **derived, never stored** — the projects whose `lastRunAt` falls within a short window of the most recent one — so it costs no new field, no new file and no new command. It renders **only** when nothing is currently non-`stopped` and the search box is empty, and it disappears the moment anything runs: it is the first ten seconds of a session, not a permanent element. Starting is N sequential `run_project` calls — no new §6 behaviour, no batching. At most a few names are listed, the rest as a count.
- **Workspace strip** (added 2026-08-10): one block at the very end of the grid, spanning the full width, after the trailing `+` tile — so it occupies **no grid cell at any project count**, landing in the empty space when the library is small and below the fold when it is large. It shows a count of projects and distinct repositories, and an inventory of the detected stack across the whole library (`TypeScript ×2 · React ×2 · Express`), deduped by project `path` and capped with a `+N`. Derived entirely from `stack`; adds no persisted state and carries no control. It must never show a status, a port, or anything that changes while a project runs — the cards own all of that.
- Empty state: "No projects yet. Add your first one." + Add button — this **is** the first-run experience (§5). Errors always say what happened and what to do next.
- **Notes:** a slide-over like the log panel, opened from the overflow menu — one free-text area per project, autosaved, Esc closes. It is a scratchpad the app never reads: nothing parses it, nothing acts on it, and it has no effect on running a project. (Added 2026-08-09.)
- **Ports** (added 2026-08-10): a slide-over like the log panel, opened from a quiet **Ports** button in the header. One row per registered project, in `projects.json` array order — never sorted by port or state, for the same reason the grid is never re-sorted. A row shows the pinned port (mono), the project name, and one of four states: **free**; the project's own §6 status, in its own colour and vocabulary, when Hangar is managing it — the only state whose row carries **Stop**, the same call the card's button makes, which never reads the displayed PID; **in use, not managed by Hangar**, whose actions are copying a `kill` command to the clipboard and — subject to §9 step 1's conditions — freeing the port; or **in use, owner unknown**, when the read-only lookup returns nothing, which says exactly that and offers no action. Whenever the port is busy and exactly one listener is identified, the row also shows that process's name, PID, start time, full command line, and a note when its parent has exited; when more than one process is listening it names none of them. It is a **snapshot, not a monitor**: it reads once on open and again only on Refresh, it never polls, and the header states when the snapshot was taken. It lists no port that is not a registered project's — a list of every listening socket would be the network inspector §3 bans. It registers, discovers or proposes nothing (§3), and opens no second OS window (§3's window-close-is-quit contract, on which §8's no-orphans logic depends).
- **Doctor** (added 2026-08-11): a slide-over like the log panel, opened from a quiet **Doctor** button in the header. One section per registered project, in `projects.json` array order — never sorted by severity, for the same reason the grid is never re-sorted and the Ports panel is never sorted by state. A section lists what Hangar can already tell, *before* a Run, about whether that project would even start: key names declared in a `.env.example` / `.env.sample` / `.env.template` that are absent from `.env`; a `.nvmrc` pin that the Node on the §8 resolved PATH does not satisfy; whether the next Run will install first (the same §9 step 3 decision, shown earlier rather than recomputed); and a project folder that is gone (§12 already warns on the card — the panel restates it so one place lists everything). A finding is one line and carries exactly four things: a stable id, a severity of **blocker · warning · note**, a human sentence, and the file it came from. A project with nothing to report says so once, quietly — no score, no badge, no percentage, because a check that celebrates itself becomes noise the user learns to skip. A project with no example env file is never asked about env keys, and one with no `.nvmrc` is never asked about Node: Hangar does not invent a policy the project never had. A missing folder, an unreadable `.env` and a malformed `package.json` are **findings, not errors** — a toast per project on open would be intolerable. It is a **snapshot, not a monitor**, on exactly the Ports panel's terms: it reads once on open and again only on Refresh, it never polls, nothing it does runs before the grid renders, and the header states when the snapshot was taken. Esc closes it on the same terms as the other slide-overs. **It reads `.env` files for KEY NAMES ONLY — a value is never retained, never serialized, never logged, never rendered, and no type in the report has a field capable of holding one.** **It carries no control that changes anything**: no fix, no install, no "create it for me", no link that writes — it reads and reports, and every remedy stays the user's own action in their own editor. And **preflight never blocks Run**: a finding never gates, delays, confirms or reorders §9, which behaves exactly as it does today whether this panel has ever been opened or not. **Dependency advisories (added 2026-08-11, plan 059) are one further source of findings and the panel's only network call**: when — and only when — the user has ticked `checkDependencies` in Settings, which is **off by default**, opening this panel sends the package names and versions from each project's `package-lock.json` to `osv.dev` and **nothing else** (no path, no project name, no machine identifier, no credential, and §3's cloud/accounts ban is otherwise untouched — this is a keyless read of a public database, not an account); packages installed from git, a local path or a link are never sent (asking about `internal-thing@1.0.0` matches either nothing or a *public* package that merely shares the name, which would report an advisory against code that is not in the project), while a package from a private registry is indistinguishable from a public one and so its name is sent — which the Settings label states; one request per project, bounded twice and never retried like §18's, capped so a monorepo cannot send an unbounded batch, and bounded a third time by a budget for the whole pass, because one slow-but-working server multiplied by ten projects would otherwise hold the panel open for minutes. **Anything not actually checked — dropped by the cap, past the budget, unreachable, unparsed — says so.** A silent omission reads as "you are clean" when it means "I did not look", and an empty finding list is indistinguishable from a clean one, so no path may return one for work that did not happen. A package with advisories is a **`warning`** — a CVE in a transitive dev dependency does not stop the project starting, and `blocker` means "will not start". Everything that is not an advisory is a `note`, never an error: a pnpm or yarn project says its lockfile was not read (parsing those needs a dependency Hangar does not carry), an npm 6 `lockfileVersion: 1` file is reported as **Hangar's** limitation and never as a broken file — blaming a valid lockfile sends people hunting a corruption that does not exist — and an unreachable database says so rather than staying quiet, because **a check that could not run must never render as a clean bill of health**. It adds no control and no fix, it is silent when clean like every other check here, and it never runs on the startup path or the §9 Run path.
- Settings: a small gear → two controls and no more: "Editor command" (default `code`), and the Doctor panel's dependency check (default **off**, added 2026-08-11 by plan 059). The second is a switch, not a field, and it carries the sentence that states exactly what leaves the machine when it is on — that sentence is part of the feature, not decoration, and must not be shortened to "we respect your privacy". Nothing else.

## 12. Edge cases (handle all)

| Case | Behavior |
|---|---|
| Port busy before spawn | Refuse to start; name the owning process and PID when the read-only lookup succeeds (§9.1) and point at the Ports panel; generic message otherwise |
| Hangar launched from Dock/Finder on macOS with nvm-managed npm | Run still finds npm via the §8 startup env resolution; if a tool is missing anyway, the log line shows the PATH searched |
| `npm` (or the command's binary) not on PATH | `crashed` + log line naming the missing tool |
| `git` not on PATH with `updateOnRun` | Log warning "git not found — skipping update", skip pull, **still start** (a missing optional tool must not fail the run) |
| `git pull` conflict / no remote / auth needed | Non-interactive env (§9.2) makes auth fail fast; log warning, skip update, still start |
| Not a git repo | Skip pull silently |
| Child exits during `starting` (exit 0 — e.g. user picked `build`) | Immediate `crashed` + "did you pick a script that starts a server?" toast; no 60 s wait |
| Child exits during `starting` (nonzero) | Immediate `crashed` + exit-code toast; log shows the real error |
| Install fails | `crashed`, no spawn, hash not stored → next Run re-installs |
| Lockfile hash matches but `node_modules` missing | Install runs anyway (§9.3) |
| Server bound to IPv6 localhost only | Detected normally — every probe is dual-stack |
| Server ready but on a different port (framework auto-bumped) | Ready-timeout → **tree killed** → `crashed` + hint: "did it start on another port? Pin it in Edit." No orphan survives the timeout |
| Stop clicked during `updating`/`installing`/`starting` | Valid: kills the active phase child, → `stopped`; a killed install never stores the hash |
| Kill verification fails | `stop-failed` (red pill), Stop button retries; never silently pretend it stopped |
| Child that daemonizes (`setsid` — Nx/Turbo daemon, watchman) | Documented limitation: outside the group/job, not Hangar's to kill (§8) |
| System sleep during `starting` | Timeout counts poll attempts, not wall-clock; sleep doesn't burn the budget |
| `projects.json` corrupt / unparseable | Never overwrite: rename to `.broken-<timestamp>`, start empty, persistent banner naming the backup (§4) |
| Project path deleted/moved | Card warning state (`pathExists`), Run disabled, Edit/Remove offered |
| Remove/Edit while running | Confirm → full kill with verification → then apply |
| Duplicate port on add/edit | Validation error naming the conflicting project |
| Same folder registered twice (different ports) | Allowed; pull/install serialized by the per-path mutex |
| Editor command not found | Toast naming the command + pointer to Settings; never silent |
| Second Hangar instance launched | single-instance plugin focuses the existing window |
| App quit while running | Confirm dialog (both interception paths, §8) → kill all trees incl. phase children → exit |
| Hangar itself force-killed | Windows: job's KILL_ON_JOB_CLOSE reaps everything; Unix: documented gap, recovery parked in §16 |

## 13. Repository layout (create in M1, do not reorganize)

```
src-tauri/src/main.rs        # builder, plugins, state, quit interception
src-tauri/src/registry.rs    # projects.json + settings.json load/save (atomic)
src-tauri/src/env_resolve.rs # §8 startup environment resolution
src-tauri/src/process.rs     # the ONE spawn helper, kill paths, log reader, ring buffers
src-tauri/src/run.rs         # §9 run sequence + state machine enforcement
src-tauri/src/commands.rs    # all #[tauri::command] fns (§7, frozen)
src-tauri/capabilities/default.json
src/types.ts                 # Project, ProjectView, Status, LogLine, event payloads — single source of truth mirroring Rust
src/api.ts                   # all invoke() wrappers (§7)
src/store.ts                 # global status + log store fed by the two event listeners
src/components/              # ProjectCard, ProjectGrid, LogPanel, AddEditDialog, PhaseStrip, SettingsDialog
src/App.tsx
```

## 14. Milestones (build in this order, one at a time)

Every milestone must pass its **Verify** gates before it is "done": `cd src-tauri && cargo check` exits 0 · `npx tsc --noEmit` exits 0 · `npm run build` exits 0 — plus the milestone-specific check.

- **M1 — Scaffold:** create-tauri-app (React/TS/Vite) + Tailwind v4 + Fontsource fonts + identifier + capabilities file; `projects.json`/`settings.json` read/write with atomic writes and corrupt-file recovery; empty state renders; a hand-added (or `HANGAR_DEV_SEED`) entry renders as a card. *Done when both the empty state and a seeded card render, and all Verify gates pass.*
- **M2 — Spawn + logs:** §8 env resolution, the shared spawn helper, ring buffers, batched `log-lines`, `status-changed`, exit-watcher; statuses transition per §6. *Done when a real dev server's output appears live in the panel and a child exit flips the card to `crashed`.*
- **M3 — Kill tree:** both platform kill paths (Job Objects on Windows), `stopping`/`stop-failed`, Stop valid in every phase, death-then-port verification, quit interception (both paths). *Done when acceptance tests 3 and 8 pass — verify on your own machine with Task Manager/Activity Monitor open.*
- **M4 — Ready + browser:** dual-stack polling racing child exit, attempt-counted timeout, grace, auto-open browser, timeout → kill → `crashed`. *Done when acceptance tests 1 and 7 pass hands-free.*
- **M5 — Add/Edit/Remove:** folder picker, script picker, port suggestions, validations, url override, confirm-and-stop, editor action + Settings. *Done when acceptance test 6 passes and a project can be added → run → edited → removed without touching the JSON by hand.*
- **M6 — Update-on-run + polish:** git pull with the non-interactive env, per-path mutex, lockfile-hash install logic, phase strip, Copy button, uptime slot, full UI pass per §11. *Done when acceptance test 5 passes and the UI matches §11 exactly.*

## 15. Acceptance tests

1. Add a real project via the dialog (setup, untimed) → from then on: click Run → browser tab shows the app, **zero keyboard input**, under 5 s after the server is ready.
2. **The switching test:** Stop project A, Run project B → B opens in a new tab; A's port is free.
3. **The orphan test:** after Stop, `node` process count returns to baseline (`tasklist | findstr node` on Windows, `pgrep -f node | wc -l` on Unix) and re-Run works immediately. Measure the baseline after one prior Run of the same project so tool daemons (Nx/Turbo/watchman — outside the kill guarantee, §8) don't pollute the count.
4. Break the project's code → Run → status `crashed` within the timeout, and the log panel shows the real error. Bonus path: point the command at a script that exits instantly → `crashed` appears immediately, not after 60 s.
5. Change a dependency in `package.json` (updating the lockfile) → next Run shows the Install phase. Delete `node_modules` → next Run also shows Install.
6. Two projects registered on the same port is impossible.
7. **The timeout-orphan test:** occupy the project's pinned port with anything else, Run (framework auto-bumps, ready-check can't succeed) → after the timeout the card is `crashed` AND the process count is back to baseline — the timed-out tree was killed, not stranded.
8. **The mid-phase stop test:** click Stop during a long `npm install` → card returns to `stopped`, the install child is dead, and the next Run re-runs Install (hash was not stored).
9. **The real test (human, two weeks):** you open Hangar instead of the terminal. If you stop doing that, the missing reason is the next feature — pulled from §16 or newly discovered, never from the OUT list.

## 16. v0.5 parking lot (deferred — build only when test 9 earns it)

Good ideas, deliberately not in v0. Each entry names the evidence that would promote it.

- **Restart action** (stop → wait port-free → run, skipping Pull/Install) — promote if two-week use shows frequent manual Stop→Run cycles. Top candidate.
- **Unix crash recovery** (`running.json` sidecar with pgid + process start-time identity checks, startup "stale processes found — kill?" dialog; never kill on PID match alone — PIDs are reused) — promote if Hangar crashes actually strand servers in practice.
- **Structured `env` field** (`env?: Record<string,string>` + textarea in Edit) — the inline `PORT=3001 npm run dev` hint covers v0; promote when a real project needs more than one variable routinely.
- **`readyCheck: "tcp" | "http"`** — promote if tabs open on blank/compiling pages (webpack-style servers that accept TCP before they can serve).
- **Stop All header button** — promote on real multi-project sessions.
- **Explicit port repin** ("Pin :5174 instead" button on the timeout toast, fed by a log regex; explicit consent, never silent) — promote if auto-bumped ports keep happening.
- **Drag-to-reorder cards** (persist by reordering the array). *Note (2026-08-10): folders consume the drag gesture. Drop **on** a tile means merge; drop **in the gap between** tiles is reserved for reorder, and the folder hit-test returns that case from day one so reorder can be added later without making existing folder drags ambiguous. This entry names no promotion evidence, so unlike every other §16 item it can only be promoted by a plain maintainer ruling.*
- **Versioned storage wrapper** (`{ schemaVersion, projects }`) — add compatibly only when a real schema change arrives (loader rule: bare array = v1).
- **Cloud-sync folder warnings** (OneDrive/iCloud/Dropbox path sniffing) — promote if sync-lock EPERM failures show up in real use.
- **System tray** — requires first deciding window-close-vs-quit; today close = quit is load-bearing for the no-orphans contract.

## 17. How to run this spec with Claude Code

1. New empty repo → save this file as `SPEC.md` → add the `CLAUDE.md` below.
2. First prompt: `Read SPEC.md fully. Build milestone M1 only. Stop when its "done when" is met.`
3. One milestone per session/prompt. Verify each "done when" yourself before continuing — especially M3 on your own machine, with Task Manager open.

```md
# CLAUDE.md
Read SPEC.md before any work. Build only the current milestone.
Scope: §3 OUT list is absolute — flag, don't build. §16 is a parking lot, not a backlog.
Rust: tokio types only, no own runtime (§4). Tauri plugins: dialog, opener, single-instance — never shell.
TS: strict, no `any`. New dependency = one-line justification comment at the import.
The §7 command/event API is FROZEN — implement subsets, never rename or reshape.
The §6 state machine and §8 process rules (one spawn helper, Job Objects, stdin null,
env resolution, kill-then-status ordering) are the highest-priority correctness requirements.
A milestone is not done until `cargo check`, `npx tsc --noEmit`, and `npm run build`
all exit 0 — run them and show the output before claiming done.
If a Tauri/plugin API doesn't match this spec's snippet, trust the compiler and current
docs, keep the spec's INTENT, and note the deviation in a code comment.
UI must follow §11 exactly — tokens, fonts, phase strip. No generic defaults.
```

## 18. GitHub integration (added 2026-08-10)

Ratified by the maintainer after being shown the cost: this is the one place §3's cloud/accounts
ban is lifted, and it is lifted **only** here. Hangar becomes an app that has a login step. Every
rule below exists to stop that changing what Hangar is for.

### The line

Hangar **surfaces** activity and lets you reply to it. It is not a GitHub client. There is no
repository browser, no file tree, no diff viewer, no PR review UI, no merge, close, label, assign,
force-push or any other destructive or state-changing operation beyond **posting a comment**. When
something needs more than a comment, Hangar opens the real page in the real browser — the same
opener path §9 step 6 already uses.

The test for any proposal here: *does it tell me something I would otherwise have missed, or is it
a worse version of a page GitHub already serves?* Build the first; link to the second.

### The launcher must not depend on it

**Hangar without a token, and Hangar with no network, must be exactly the app it is today.** Run,
Stop, logs, ports, folders, the phase strip — none of them may acquire a network dependency, block
on one, or slow down because one is pending. A GitHub failure is never a Run failure. If this rule
and a feature conflict, the feature loses.

Concretely: no GitHub call may run on the §9 run sequence's path, hold any lock §8 or §9 uses, or
appear in the startup path before the grid renders.

### The credential

- **One** credential: a GitHub personal access token, supplied by the user.
- It is a **secret**. It is never written to `projects.json`, never logged, never included in a
  toast, an error string, a `system` log line, or a panic message, and never sent anywhere except
  `api.github.com` over TLS.
- It is stored via the **OS keychain**, not a JSON file. If the keychain is unavailable, the
  feature is unavailable — it does not silently fall back to disk.
- Removing it must be one obvious action, and must leave no residue.
- The token's scopes are the user's choice; Hangar states which it needs and requests no more.

### Network behaviour

- **Never polls in the background.** §3 still bans background mode. Fetch on: the inbox being
  opened, an explicit Refresh, and at most once on window focus with a sane minimum interval.
- **Rate limits are shown, never swallowed.** When GitHub throttles, the UI says so and says when
  it resets.
- **Offline is a first-class state**, not an error banner. The inbox says it is offline and shows
  the last snapshot with its timestamp — the same staleness-is-visible rule §5 applies to
  `stack.detectedAt`.
- Every request has a timeout. A hung request may never wedge the UI.

### Storage

- Cached GitHub data is a **cache**, not a database: a single JSON file, atomic writes like every
  other file in §4, safe to delete at any moment, and never the source of truth for anything.
- It holds no secret and no PII beyond what the API returned for the repositories the user linked.
- `projects.json` gains at most a repository identifier per project. Nothing else.

### Scope of the first version

- An **inbox** of notifications for linked repositories, with unread state.
- Reading an issue or PR thread.
- **Posting a comment.** Nothing else writes.
- Opening the corresponding page on github.com.

Everything else — reviews, merges, labels, assignments, releases, Actions, code — is out, and stays
out unless this section is amended again.

### What this section does not license

It is not a precedent for a second provider, a second credential, telemetry of any kind, an
embedded browser, or any other §3 entry. Those remain ❌ exactly as written.
