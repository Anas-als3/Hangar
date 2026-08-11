# Plan 063: Tell the user when they are looking at an old build

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise. Update this plan's row in
> `plans/README.md` when done.
>
> **Drift check**: `grep -n "ResumeLine" src/App.tsx && grep -n "LaunchLine" src/App.tsx`
> Both must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo check
> --no-default-features --all-targets`, `cargo test` and `npm run typecheck`.
> Your reviewer runs `npm run build` and the bundle.

## Status

- **Priority**: P1
- **Effort**: **S**
- **Risk**: LOW
- **Depends on**: nothing
- **Category**: DX / bug-class elimination
- **Planned at**: 2026-08-11

## The problem, with its cost

Hangar is developed on the machine it runs on, and it is installed to
`/Applications`. So the loop is: change code → `npm run install:app` → the
`.app` on disk is new, **but the running process is not.** macOS keeps a
running app's code in memory; replacing the bundle does nothing to the window
already open.

**This has now cost the maintainer three separate confusions**, the most recent
on 2026-08-11: a feature merged, installed at 09:11, and reported missing
because the window open in front of him had been running since 06:54. Each time
the conclusion looked like *"the feature was not built"* — the most expensive
possible wrong conclusion, because it sends someone debugging code that is
correct.

It also wasted a real diagnostic detour: the first check run against it was a
`grep` of the installed binary for a UI string, which returned zero — and would
have returned zero anyway, because Tauri compresses embedded frontend assets.
A control string proved the method useless. **The app should just say.**

## What to build

A quiet line, in the same family as the resume line and the launch line:

> **A newer build is installed. Restart Hangar to use it.**

Rendered **only** when the installed bundle is newer than the running process.
Silent otherwise — zero pixels, and on any machine where the app was not
installed from a local build, it is silent forever.

## How to decide "newer" — get this right

Two facts, both cheap:

1. **When this process's executable was built or installed.** Prefer a value
   baked in at compile time (a build timestamp via `build.rs` or an
   `env!("CARGO_PKG_VERSION")`-style constant) over reading the running binary's
   mtime — an mtime moves when the file is copied, which is exactly what
   `install:app` does, and would make a fresh install look stale.
   **Say which you chose and why.**
2. **The mtime of the installed bundle's executable on disk.**

If (2) is meaningfully newer than (1), the running process is stale.

**Use a tolerance, not a strict `>`.** Filesystem timestamps and a compile-time
constant do not share a clock origin, and a few seconds of skew must not
produce a permanent nag. Pick a threshold, justify it in a comment, and test
both sides of it.

## The rules

- **Never on the network.** Two filesystem stats and a constant.
- **`Ok`, never `Err`.** If the bundle path does not exist, is unreadable, or
  the app was launched from somewhere unexpected, the answer is **"say
  nothing"** — not an error, not a toast. This runs on the startup path.
- **It must never claim to be stale when it is not.** A false nag teaches the
  user to ignore the line, and then it is worse than absent. When in doubt,
  stay silent.
- **No auto-restart, no auto-update, no "Restart now" button that kills
  processes.** §3 bans auto-update, and §8's whole guarantee is that Hangar
  owns its children's lifecycle — a restart button that silently killed a
  running dev server would be a §6/§8 violation wearing a convenience hat.
  **Text only. The user restarts.**
- If you want to offer more than text, the **most** it may do is the same thing
  the launch line does: nothing but inform. Confirm with the maintainer before
  going further.

## Scope

**In scope**:
- `src-tauri/build.rs` **or** an equivalent — the build-time constant.
- `src-tauri/src/` — one small module, or an addition to an existing one. Pure
  comparison function, tested without touching a filesystem.
- `src-tauri/src/commands.rs` — one command, or fold into an existing startup
  read. Prefer folding; say which.
- `src/components/` — the line; model it on `ResumeLine`.
- `SPEC.md` §11 — one short entry.

**Out of scope**:
- Auto-update, update download, version checking against a server. §3.
- Any restart, kill, or process action.
- Any change to `run.rs`, `process.rs`, §6, §8, §9.
- Windows and Linux specifics. **macOS only for now** — the maintainer has said
  Windows is not a priority. On other platforms the check returns "say
  nothing", and a comment records that this is a deliberate stub, not an
  oversight.

## Testing

The comparison is a pure function of two timestamps and a tolerance. Test:
1. Installed newer by well past the tolerance → stale.
2. Installed newer by less than the tolerance → **not** stale.
3. Installed older → not stale.
4. Either value missing → not stale.

Then **mutation-test case 2**: remove the tolerance, confirm the test goes red,
restore. A one-second skew producing a permanent nag is the realistic failure
here, and it is the one users actually feel.

## Done criteria

- [ ] Four gates green; `cargo test` before/after reported
- [ ] The tolerance test was mutation-tested — report both outcomes
- [ ] No network call in the diff
- [ ] No restart/kill control anywhere
- [ ] The line renders nothing on a normally-launched, current app
- [ ] `plans/README.md` row updated

## STOP conditions

- You are about to add a "Restart now" button that terminates the process.
  Stop — that is §8's territory and needs the maintainer's decision.
- The check needs a network call or a version server. It does not.
- You cannot find a build-time constant that survives `install:app`'s copy.
  Report what you tried rather than falling back to something that produces
  false positives.

## Maintenance notes

- The failure mode to watch is the **false nag**. If this line ever shows on a
  freshly restarted app, it is worse than not existing, because the next real
  one will be ignored too.
- This is a developer-machine feature. On a normal user's machine it should
  never render — which is also why it must fail silent rather than loud.
