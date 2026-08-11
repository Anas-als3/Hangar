# Plan 047: Put a Ports button on the port-collision toast

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise. Update this plan's row in
> `plans/README.md` when done, unless a reviewer told you they maintain it.
>
> **Drift check**: `grep -n "is in use by" src-tauri/src/run.rs && grep -n "openPorts" src/store.ts`
> Both must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. Your reviewer runs `npm run build`.

## Status

- **Priority**: P2 — closes a requirement the spec already carries
- **Effort**: S
- **Risk**: LOW
- **Depends on**: plan 041 (DONE)
- **Category**: bug
- **Planned at**: commit `bcc8233`, 2026-08-10

## Why this matters

SPEC.md §12's "Port busy before spawn" row was amended on 2026-08-10 to read:

> Refuse to start; name the owning process and PID when the read-only lookup
> succeeds (§9.1) **and point at the Ports panel**; generic message otherwise.

`src-tauri/src/run.rs` names the PID and points nowhere. **The requirement was
written and never built** — by me, the same day. The maintainer hit this exact
failure twice in one day.

Everything needed already exists: the toast carries `projectId` (plan 034),
`App.tsx` already renders a project-scoped button off it ("Show logs"), and
`openPorts()` is already exported from the store (plan 041).

## The constraint that decides the design

`run_project` returns `Result<(), String>` — §7 is FROZEN, so the frontend
receives **a plain string** and cannot tell a port refusal from an install
failure. **Do not regex the message.** A regex would put a Ports button on
unrelated toasts and would break the moment the wording changes.

Instead: give the toast an explicit, typed reason. Add an optional
`toastAction` to the store's toast state — a small discriminated union, not a
parsed string — and set it at the one call site that knows the Run was refused
for a busy port.

**The frontend does not know that either.** So the honest options are:

1. Have Rust classify it. `run_project`'s error string is built in one place
   for the busy-port case — but §7 freezes the return type, so it cannot carry
   a code.
2. Have the **backend emit a `system` log line and the frontend key off nothing**
   — no.
3. **Recommended:** the port pre-check already runs in `run.rs` *before* the
   spawn. Emit the existing `status-changed`-style information another way:
   the frontend calls `openPorts()` from a button that is shown whenever the
   toast has a `projectId` **and** that project is `stopped`/`crashed` — i.e.
   the Run did not take. That is not a regex and it is not a lie: "see this
   project's port" is useful for every failed Run, not only a collision.

Pick option 3 unless you find something better while reading; if you do,
report it rather than inventing a fourth.

## Scope

**In scope**: `src/App.tsx` (the `Toast` component), `src/store.ts` (only if a
selector is needed).

**Out of scope**: `src-tauri/` entirely — §7 is frozen and this needs no
backend change. `PortsPanel.tsx`, the Ports fetch logic, the log panel, the
card. No new dependency.

## Steps

### Step 1: The button

In `App.tsx`'s `Toast`, add a **Ports** button beside the existing "Show logs",
shown when `toastProjectId` resolves to a project **and** that project's status
is `stopped` or `crashed`.

It calls `openPorts()` then `setToast(null)` — same shape as the Show-logs
button, which dismisses so the toast does not sit over the panel it opened.

Styling: identical to Show logs. It must not become the visually dominant
action; Dismiss stays the simplest.

**Verify**: `npm run typecheck` → 0.

### Step 2: Self-check

- `grep -rn "openPorts" src/App.tsx` → present.
- `grep -rn "in use by\|match(\|RegExp" src/App.tsx` → **no message parsing**.
- `git diff --stat` → `src-tauri/` untouched.

**Verify**: `npm run typecheck` → 0; `cargo check --all-targets` → 0;
`cargo test` → all pass (unchanged count — you changed no Rust).

## Test plan

Manual, for the reviewer/maintainer:

- Start a dev server outside Hangar on 5173, then press Run on Example App.
  The toast names the PID **and** offers **Ports**; pressing it opens the panel
  with that row visible.
- A crash toast (not a port collision) also offers Ports — intended, and
  harmless: the row shows the port free.
- A toast with no project (a generic error) offers neither button.

## Done criteria

- [ ] `npm run typecheck` 0; `cargo check --all-targets` 0; `cargo test` unchanged
- [ ] No message-string parsing anywhere
- [ ] Nothing under `src-tauri/` modified
- [ ] `plans/README.md` row updated

## STOP conditions

- You find yourself regexing the toast text. Report instead — that is the trap
  this plan exists to avoid.
- The button seems to need a new §7 command or a changed return type. §7 is
  frozen and this needs neither.

## Maintenance notes

The general lesson: `run_project` returning a bare `String` means the frontend
can never distinguish failure *kinds*. If a third failure ever needs its own
affordance, that is the moment to propose a §7 addition carrying a reason code
— not a third regex.
