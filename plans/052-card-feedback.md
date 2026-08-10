# Plan 052: A card that tells you what just happened

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise. Update this plan's row in
> `plans/README.md` when done, unless a reviewer told you they maintain it.
>
> **Drift check**: `grep -n "one additional element" SPEC.md && grep -n "runDisabled" src/components/ProjectCard.tsx`
> Both must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `npm run typecheck`. Your reviewer runs
> `npm run build` and `cargo check`. **This plan touches no Rust.**

## Status

- **Priority**: P2 — two silences, both hit on the maintainer's real registry
- **Effort**: S
- **Risk**: LOW-MED — one part adds card state the backend does not own, which
  is exactly what §7's "derive all status UI from `status-changed`" guards
  against; read the constraint below carefully
- **Depends on**: SPEC.md §11 crash-reason amendment (ratified 2026-08-10)
- **Category**: bug
- **Planned at**: commit `bcc8233`, 2026-08-10

## Why this matters

### Part A — the Run button does not admit it was clicked

`run.rs` takes the per-path mutex **before** it claims a §6 status. When two
cards share a folder — the maintainer's `auto-job-applier` pair, both with
`updateOnRun: true` — the second Run blocks for the whole pull + install of the
first. During that time the second card shows `Stopped`, renders an unlit phase
strip, and keeps its **Run button enabled**. Clicking does visibly nothing.

Worse: each extra click queues another `run_project` task, and each is rejected
a minute later with *"… is starting — Run is only valid from stopped or
crashed"* — a pile of confusing toasts for a project the user did nothing wrong
with.

### Part B — a crashed card does not say why

A crash shows a red `Crashed` pill and a toast. The toast is one overwritable
slot; the reason is otherwise two clicks behind the overflow menu. §11 was
amended for this.

## The constraint that governs Part A

§7 says all status UI derives from `status-changed`. This adds a state the
backend does not own, so it must be unmistakably **not a status**:

- It is a property of **the click**, not of the project.
- It **never renders a §6 status name**. The button reads `Starting…` — the
  same word the existing `stopping` spinner uses — and the **status pill does
  not change at all**.
- It is cleared by the **first real `status-changed`** for that project, or when
  the `run_project` invoke settles, whichever happens first.
- It never persists, never survives a reload, and is never read by anything
  except that one button.

Write that reasoning into the code as a comment, not only into this plan. Get
it wrong and you have re-invented optimistic status.

## The constraint that governs Part B

§11's amendment is explicit: source the text from **the `status-changed`
event's `message`**, never from the last line of the log buffer.

`crash_run` passes its message (e.g. `Install failed (exit 1) — see the log,
then Run again.`) to the status event **only** — it never enters the buffer. A
last-line heuristic would print an unrelated earlier warning (`git not found —
skipping update`) under a red pill as though it were the cause.

The store already receives that message and spends it on a toast. Keep it in an
ephemeral map beside `phasesSeen` — the plan-027 precedent — never persisted.

## Scope

**In scope**: `src/store.ts` (two ephemeral maps), `src/components/ProjectCard.tsx`.

**Out of scope**: anything under `src-tauri/`; the status pill's colours or
labels; `phasesSeen` itself; the phase strip; the toast; the folder tile; the
drag subsystem; `run.rs`'s messages. No new dependency, no persisted state.

## Steps

### Step 1: Pending-run state

In `store.ts`, add an ephemeral `pendingRun: Record<string, true>` (or a `Set`
kept in the same immutable style as `openFolders`).

- `startProject` marks the project pending **before** the invoke.
- It clears in a `finally`, and `applyStatusChanged` clears it for that project
  on any incoming status.
- `loadRegistry` / `refreshRegistryQuietly` must **not** touch it.

**Verify**: `npm run typecheck` → 0.

### Step 2: The button reflects it

In `ProjectCard.tsx`, when a project is `stopped`/`crashed` **and** pending,
the primary button is disabled and reads `Starting…` with the existing spinner
markup — copy the `stopping` branch's shape exactly.

**The status pill is untouched.** It still reads `Stopped`. That asymmetry is
the design: the pill is the backend's truth, the button is about your click.

**Verify**: `npm run typecheck` → 0.

### Step 3: Crash reason

In `store.ts`, add an ephemeral `lastFailure: Record<string, string>`:

- Set in `applyStatusChanged` when the status is `crashed` or `stop-failed`
  **and** the payload carries a message — the same condition that already fires
  the toast.
- Cleared for that project on any other status (a fresh run wipes it).
- Never persisted; never touched by `loadRegistry`.

**Verify**: `npm run typecheck` → 0.

### Step 4: The card line

In `ProjectCard.tsx`, when the project is `crashed` or `stop-failed` **and**
`lastFailure` has an entry, render one muted line:

- `truncate`, with the full text in `title`.
- Clicking it opens that project's log panel (`openLogs`) — the reason and the
  detail belong together.
- Place it where the libraries line sits relative to the status row — a sibling,
  never a fourth item inside the status flex container.
- No new colour. Muted text; the red pill already carries the severity.

**Verify**: `npm run typecheck` → 0.

### Step 5: Self-check

- `grep -n "pendingRun\|lastFailure" src/store.ts` → both present; neither in
  `loadRegistry` or `refreshRegistryQuietly`.
- `grep -rn "pendingRun\|lastFailure" src-tauri/` → **no matches**.
- `grep -n "STATUS_LABEL" src/components/ProjectCard.tsx` → the pill still
  renders the real status, unmodified.
- `git status --short` → only the two in-scope files.

**Verify**: `npm run typecheck` → 0.

## Test plan

No JS test runner for component behaviour. Manual, for the reviewer/maintainer:

- Press Run on **both** `auto-job-applier` cards quickly. The second shows
  `Starting…` on its button immediately while its pill still reads `Stopped`,
  and further clicks do nothing. Before this, it looked idle and each click
  queued a rejected run.
- Force an install failure. The card shows the reason under the status row;
  clicking it opens the log.
- Run it again → the line clears.
- Crash a project, then search it out and back → the line survives (it lives in
  the store, like `phasesSeen`).

## Done criteria

- [ ] `npm run typecheck` 0
- [ ] The status pill is unchanged in every state
- [ ] Neither map is persisted or touched by a registry reload
- [ ] The crash line comes from the event message, never the log buffer
- [ ] Nothing under `src-tauri/` modified
- [ ] `plans/README.md` row updated

## STOP conditions

- The pending state seems to want to render a §6 status name, or to live in the
  pill. Both are the failure mode this plan is written to avoid.
- The crash reason seems easier to read from the log buffer. It is not there —
  re-read "The constraint that governs Part B".
- Either map seems to need persisting. Both are ephemeral by design.

## Maintenance notes

- `pendingRun` covers **every** pre-status delay — the port probe, `git
  rev-parse`, the path mutex — not just the shared-folder case that motivated
  it. That is why it is keyed on the click rather than on any one cause.
- If a third ephemeral per-project map appears, that is the moment to consider
  one `Record<string, CardEphemera>` instead of three parallel maps.
