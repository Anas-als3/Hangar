# Plan 041: The Ports panel — see what is holding a port without leaving Hangar

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "fn port_owner\|fn parse_lsof_owner\|fn port_accepts" src-tauri/src/process.rs`
> All three must exist. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. **Your reviewer runs `npm run build` and the bundle.**

## Status

- **Priority**: P2 — twice in one day the maintainer left Hangar to answer a
  question Hangar already had the machinery to answer
- **Effort**: M
- **Risk**: MED — new §7 command, new over-the-grid surface, forks child
  processes
- **Depends on**: SPEC.md §7/§11/§12 amendments (ratified 2026-08-10)
- **Category**: feature
- **Planned at**: 2026-08-10

## Why this matters

Twice today Hangar refused a Run, correctly, and said:

> Port 5173 is in use by node (PID 57140) — is this project running elsewhere?

Both times the maintainer left the app — **not to kill anything, but to find
out what PID 57140 was.** `node (PID 57140)` is not enough to decide anything.
What actually resolved it was the full command line
(`…/scratchpad/fix-a11y/node_modules/.bin/vite` → "ah, another session"), the
start time, and the fact that its parent had exited.

This panel shows those three facts for the ports Hangar already knows about.
The lookup already exists (`process.rs`'s `port_owner`); it is simply
unreachable outside a refused Run's error string.

**Read SPEC.md §11's "Ports" bullet and the amended §7 block before starting.**
Both were written for this plan.

**A separate plan (042) adds the "Free the port" action.** This plan ships the
panel and a **Copy `kill <pid>`** button. Do not build the kill here.

## Why the panel and not a button on the toast

§7 freezes `run_project(id): void` returning `Result<(), String>`, and
`store.ts` turns the refusal into `setToast(errorMessage(err), "error", projectId)`
— **a plain string**. The frontend cannot distinguish a port refusal from an
install failure without regexing the message text. The panel calls
`get_port_status()` and has the PID as structured data.

## Why a slide-over and not a window

`src-tauri/src/main.rs` matches `WindowEvent::CloseRequested` with **no label
filter**, routing to the quit path. **Closing a second window would quit
Hangar.** And `capabilities/default.json` lists `"windows": ["main"]`, so a
second window gets no ACL and cannot `invoke` at all.

## Current state

`src-tauri/src/process.rs`:

- `port_accepts(port) -> bool` (:532) — already dual-stack (IPv4 **and** IPv6),
  which §12 requires.
- `PORT_OWNER_TIMEOUT` (:553) — 2 s.
- `port_owner(port, env) -> Option<PortOwner>` (:576) — runs `lsof` through the
  one spawn helper.
- `parse_lsof_owner(stdout) -> Option<PortOwner>` (:628) — returns the **first**
  parseable row, on the documented assumption that dual-stack rows are one
  process. **That assumption is fine for a toast garnish and not fine here** —
  see step 2.

Existing slide-overs to model on: `src/components/LogPanel.tsx` and
`NotesPanel.tsx`. Read both.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --all-targets` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass (baseline 127 — **run it first and report what you observe**) |
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run build`, `npm run verify`, `npm run build:app`,
`npm run install:app`, or `npm run test:acceptance`. Keep every Write/Edit
under ~60 lines and commit after each.

## Scope

**In scope**:
- `src-tauri/src/process.rs` — an all-rows lsof parser and a batched `ps` read
- `src-tauri/src/commands.rs` — `get_port_status`, and its registration
- `src-tauri/src/registry.rs` — the drift-guard samples only
- `src/types.ts` — `PortStatus` / `PortHolder`
- `src/api.ts` — the one new `invoke`
- `src/store.ts` — `portsOpen` view state and the fetch action
- `src/components/PortsPanel.tsx` — new
- `src/App.tsx` — mount it; add the header button; extend the `inert` overlay set
- `src/components/ProjectGrid.tsx` — the folder band's Esc guard

**Out of scope** (do NOT touch):
- **Any kill, signal, or `free_port`.** Plan 042 owns that entirely. If you
  write code that sends a signal, you have exceeded scope — STOP.
- `port_owner`, `port_accepts`, `PORT_OWNER_TIMEOUT`, the spawn helper, §9's run
  sequence, §8's kill paths, the §6 state machine. Reuse; do not modify.
- `parse_lsof_owner` itself — **leave it exactly as it is**. Its single-row
  behaviour is depended on by the toast path and its tests. Add a new function
  beside it.
- Polling of any kind. No `setInterval`, no timer, not even while the panel is
  open.
- Listing ports that are not a registered project's.
- Any new dependency.

## Git workflow

- One commit per step: `Ports: <what>`.

## Steps

### Step 1: The wire types

`src-tauri/src/commands.rs` (or `registry.rs` if that is where sibling payload
structs live — read and match): `PortStatus` and `PortHolder` exactly as §7's
amended block declares them, `#[serde(rename_all = "camelCase")]`, with
`Option` fields `skip_serializing_if = "Option::is_none"`.

Mirror both in `src/types.ts`.

**The drift guard needs feeding.** `registry.rs`'s
`every_wire_key_the_backend_emits_appears_in_types_ts` only checks the samples
it is given. Add a fully-populated `PortStatus` (with `holder: Some(...)`, every
`Option` field `Some`) to its sample vec — an omitted struct lets the guard pass
while verifying nothing, and a `None` field is invisible to it because
`skip_serializing_if` omits the key.

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass. Then
mutation-test the guard: delete `listenerCount` from `types.ts`, confirm the
guard **fails**, restore it, confirm it passes. **Report both outcomes.**

### Step 2: An all-rows lsof parser

Add a **new** function beside `parse_lsof_owner` that returns **every** distinct
PID in the output, not the first.

Why this is not a refinement of the existing one: a dual-stack server produces
two rows for one process, which is what the current parser assumes — but two
*different* processes on `127.0.0.1:P` and `[::1]:P` is legal, and
`port_accepts` is `v4 || v6`. So the named PID may not even be the one blocking
the Run. The panel must be able to say "2 processes are listening — Hangar will
not guess which one", and plan 042's safety gate depends on knowing the count.

Also parse the **USER** column (lsof field 3) so `sameUser` can be populated.

Unit-test it against: a single-process dual-stack pair (→ one PID); two distinct
PIDs (→ two); a header-only output (→ none); a malformed row (→ skipped, not an
error).

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass, report the
new total.

### Step 3: `get_port_status`

A `#[tauri::command]` returning `Vec<PortStatus>`, one entry per registered
project, in `projects.json` array order.

Lock discipline — **this is the part that must not be got wrong**:

1. Snapshot `(id, port)` for every project **under the lock**.
2. **Drop the lock.**
3. Probe and look up.

§4 forbids holding the async mutex across a long await, and `lsof` is one. Plan
010's maintenance note says the same. Follow the shape `run.rs` already uses:
it drops its path guard *before* reading.

For each project: `port_accepts(port)`. If busy, run the existing `lsof` lookup
and the new parser. When exactly one PID is found, enrich it with **one batched**
`ps -o pid=,ppid=,lstart=,command= -p <pid>[,<pid>…]` through the same spawn
helper — one child for all rows, not one per row. `command=` must come **last**;
it contains spaces.

`parentExited` is `ppid == 1`. `sameUser` compares lsof's USER against the
current user. `checkedAt` is an ISO timestamp via the existing `iso8601_utc`.

A lookup that fails or times out yields `busy: true, listenerCount: 0, holder: None`
— the "owner unknown" state. **It is never an error**: this command must not
return `Err` for a project whose owner could not be identified.

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass;
`grep -n "lock()" src-tauri/src/commands.rs` — confirm in your report that no
`lsof`/`ps` await happens between a `lock()` and its drop.

### Step 4: Store and API

- `src/api.ts`: one `invoke("get_port_status")`.
- `src/store.ts`: `portsOpen: boolean` and `ports: PortStatus[] | null` in
  `HangarState`; `openPorts()` (sets open, fetches), `closePorts()`,
  `refreshPorts()`. Errors set a toast and leave the previous rows.
- **Ephemeral only.** Nothing persisted, and `loadRegistry` /
  `refreshRegistryQuietly` must never touch these fields.

**Verify**: `npm run typecheck` → 0.

### Step 5: The panel

`src/components/PortsPanel.tsx`, modelled on `LogPanel.tsx` — read it first and
match its shell, backdrop, and Esc handling idiom.

Header: `Ports` · `as of 14:31:07` (mono) · a **Refresh** button · `✕`.

One row per project, in array order, showing the pinned port (mono), the project
name, and one of four states:

| State | Condition | Action |
|---|---|---|
| `Free` | not busy | none |
| the project's own §6 status label + tone | project status is non-idle | **Stop** |
| `In use — not managed by Hangar` | busy, project is `stopped`/`crashed` | **Copy `kill <pid>`** |
| `In use — owner unknown` | busy, `listenerCount === 0` | **none, ever** |

Reuse `STATUS_LABEL` / `STATUS_TONE` from `src/status.ts` for the managed state —
do not invent a second vocabulary. The **Stop** button calls the existing
`stopProjectAction(project.id)` and **must never read the displayed PID**; that
is the structural safety property that keeps this panel honest.

**The holder block renders whenever the port is busy and exactly one listener
parsed — in every state, including Hangar's own.** That is what makes this a
diagnostic rather than a status mirror: when the two agree it is a harmless
confirmation; when they disagree you see it immediately.

```
node · PID 57140 · started 13:53 today
/private/tmp/…/scratchpad/fix-a11y/node_modules/.bin/vite    ← mono, truncate + title
its parent has exited — nothing is supervising it            ← only when parentExited
```

Exact strings:
- `listenerCount > 1`: `2 processes are listening on this port — Hangar will not guess which one.` No copy button.
- `listenerCount === 0` while busy: `Hangar could not identify the owner. Processes owned by another user or by the system are not visible to it.`
- Copy button: `Copy kill 57140` → clipboard gets `kill 57140`; shows `Copied` ~1.5 s. Use the same
  `navigator.clipboard.writeText` + `execCommand('copy')` fallback the log panel's Copy button uses.

**No polling.** Fetch on open and on Refresh only. Staleness is visible in the
header — the same principle §5 applies to `stack.detectedAt`.

**Verify**: `npm run typecheck` → 0.

### Step 6: Wire it in

- `src/App.tsx`: a quiet **Ports** button in the header beside the running
  count, hidden when the registry is empty; mount `<PortsPanel />` as a sibling
  **after** the `inert` wrapper, alongside the other overlays; add `portsOpen`
  to the `overlayOpen` expression.
- `src/components/ProjectGrid.tsx`: the folder band's Esc guard currently reads
  `!openLogsFor && !notesFor && !dialog`. **Add `&& !portsOpen`**, or one Esc
  closes both the panel and an open folder — the exact defect plan 033 fixed
  once already.

**Verify**: `npm run typecheck` → 0.

### Step 7: Self-check

Report each:

- `grep -rn "kill\|signal\|SIGTERM" src-tauri/src/commands.rs` → nothing new from you.
- `grep -n "setInterval\|setTimeout" src/components/PortsPanel.tsx` → only the ~1.5 s "Copied" reset.
- `grep -n "portsOpen" src/components/ProjectGrid.tsx` → present in the Esc guard.
- `grep -n "parse_lsof_owner" src-tauri/src/process.rs` → unchanged, plus your new function beside it.
- `git status --short` → only in-scope files.

**Verify**: all three gates green.

## Test plan

Rust unit tests: the new parser (step 2's four cases). The `ps` enrichment
should be split so its **parsing** is testable without spawning.

Manual checks for the reviewer/maintainer:

- Open Ports with all three projects stopped and nothing else listening → three
  `Free` rows.
- Run one project → its row shows `Running` and a **Stop** button; pressing it
  stops the project, exactly as the card's button does.
- Start a dev server outside Hangar on 5173, then open Ports → the row reads
  `In use — not managed by Hangar`, shows the command line and start time, and
  offers `Copy kill <pid>`. Paste it in a terminal; it works.
- Press **Refresh** after killing it → the row flips to `Free` and the header
  timestamp updates.
- Esc with a folder open and the panel open → **only the panel closes.**
- Tab while the panel is open → focus never reaches a Run button behind it.

## Done criteria

- [ ] All three gates green; report `cargo test` before/after
- [ ] The drift-guard mutation test was run and both outcomes reported
- [ ] No signal is sent anywhere; no `free_port`
- [ ] No polling; no port listed that is not a registered project's
- [ ] `parse_lsof_owner` unchanged
- [ ] No `lsof`/`ps` await inside a held lock
- [ ] `plans/README.md` status row for 041 updated

## STOP conditions

Stop and report back if:

- You find yourself writing a kill, a signal, or `free_port`. Plan 042 owns it,
  and it has safety gates this plan does not describe.
- A second OS window seems easier than a slide-over. It would quit the app on
  close — see above.
- The lookup needs to run while a lock is held. Report the constraint instead.
- Listing every listening socket seems more useful. §3 bans a network
  inspector, and a non-root `lsof` sees only the current user's listeners, so
  such a list would be **silently incomplete** — worse than not offering it.

## Maintenance notes

- The safety property that matters is structural, not attributional: **the only
  button that acts is Stop, and Stop takes a project id, never the displayed
  PID.** Keep it that way — plan 042 adds an action that does take a PID, and it
  carries its own gates for exactly that reason.
- `kill_pid`-based attribution was considered and rejected: it is a documented
  recycled-pgid risk, it is cleared at `begin_run`, and it is `None` for the
  `stop-failed` case that most needs it. Row state comes from the project's §6
  status instead.
- If the registry ever grows past ~10 projects, swap the per-port `lsof` for one
  machine-wide call filtered in Rust — under the standing rule that **the wide
  read never leaves Rust**, or the panel becomes the network inspector §3 bans.
