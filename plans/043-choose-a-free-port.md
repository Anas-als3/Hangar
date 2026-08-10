# Plan 043: "Choose for me" — pick a free port *and* tell the project about it

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "find_free_port" SPEC.md && grep -n "sniff_port_suggestion" src-tauri/src/registry.rs`
> Both must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. **Your reviewer runs `npm run build` and the bundle.**

## Status

- **Priority**: P3 — maintainer-requested; the friction is real but small
- **Effort**: M
- **Risk**: MED — a wrong port token means a 60-second wait and then Hangar
  kills a healthy server
- **Depends on**: SPEC.md §7/§10 amendments (ratified 2026-08-10)
- **Category**: feature
- **Planned at**: 2026-08-10

## Why this matters, and the fact that shapes the whole design

The maintainer: *"add a feature to make it no need to choose a port number and
just picks whatever port available"*.

**Hangar does not choose the port. The project's own config does.** Hangar runs
the command through a shell and passes no port; everything it does with `port`
is *watching* — the §9 step 1 pre-check, the ready poll, the browser URL, Stop's
verification, and duplicate-port validation. Six observers, zero inputs.

So "pick a free port" can only mean "**wait on** a different number", and
whether the server agrees is decided by the project, not by us. Verified across
the maintainer's own three registrations:

| Project | Would honour an injected `PORT`? |
|---|---|
| IELTS Coach — `vite.config.ts`: `port: Number(process.env.PORT) \|\| 5173` | **Yes** — but only because a human wrote that expression |
| auto-job-applier web — `web/vite.config.ts`: `port: 5173` (bare literal) | **No.** Vite core reads no `PORT` |
| auto-job-applier server — `config.ts`: `Number(process.env.PORT ?? 4000)` | **Yes** |

Two of three would work; one would fail on **100 % of runs, forever, at 60
seconds a time**, and Hangar would kill a perfectly healthy server to get there.
Hangar cannot tell them apart from outside — the difference is a hand-written
expression inside a TypeScript file.

**Therefore the button writes the port token into the command in the same
press.** That automates exactly the workaround the maintainer already applies by
hand (`npm run dev --workspace web -- --port 5175`). Read SPEC.md §10 step 4;
it was amended for this and says the two halves are inseparable.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --all-targets` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass (baseline 143 — **run it first and report what you observe**) |
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run build`, `npm run build:app`, `npm run install:app`,
`npm run verify`, or `npm run test:acceptance`. Keep every Write/Edit under ~60
lines and commit after each.

## Scope

**In scope**:
- `src-tauri/src/commands.rs` — `find_free_port`, registration
- `src/types.ts`, `src/api.ts` — the one new call
- `src/components/AddEditDialog.tsx` — the control, the token rewrite, the caption
- A new pure module for the token rewriting (see step 3) **plus its `node --test` file**

**Out of scope** (do NOT touch):
- **`sniff_port_suggestion`.** The prefill stays a *framework prediction*.
  §10 step 4's amendment says so explicitly: bumping the suggestion to 5174
  because 5173 is momentarily busy would make the very first Run of a Vite
  project fail. Availability enters **only** through the explicit button.
- `port_conflict`, `get_port_status`, `free_port`, `port_owner`, `port_accepts`,
  the §9 run sequence, the §6 state machine, `run.rs`, `process.rs`'s spawn/kill.
- Injecting `PORT` into the child's environment. That is one line in `run.rs`
  and it would kill a healthy Vite on `auto-job-applier web` on every run.
- Reading the port back from log output. §3 bans it outright.
- Any per-run port selection. §5 says `port` is pinned per project.
- Any new dependency.

## Git workflow

- One commit per step: `Port picker: <what>`.

## Steps

### Step 1: `find_free_port`

A `#[tauri::command]`: `find_free_port(from: u16, exclude: Vec<u16>) -> Option<u16>`.

- Walk upward from `from`, at most **20** candidates.
- Skip any port in `exclude` (the frontend passes the ports other registered
  projects pin) and any port that currently accepts a connection — reuse
  `process::port_accepts`, which is already dual-stack as §12 requires.
- **Never fall back to returning `from` when the walk is exhausted.** Return
  `None`. A caption claiming a port is free when the walk never proved it is the
  exact silent magic §10 step 4 forbids.
- Same lock discipline as `get_port_status`: if you need registry data, snapshot
  under the lock and probe after releasing it.

Register in `main.rs`. Mirror the signature in `src/types.ts` / `src/api.ts`.

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass.

### Step 2: Wire the control

In `AddEditDialog.tsx`, beside the Port field:

- **Framework known** (`stack.framework` is set) → one button: `Choose for me`.
- **Framework unknown** → two: `Choose for me: [--port] [PORT=]`, with a helper
  line: `Hangar can't tell which this project reads. --port suits Vite, Next, Astro, Nuxt, SvelteKit and Angular; PORT= suits a plain Node/Express server.`
- **Disabled while `path` is empty** — the walk has nothing meaningful to run
  against.

One press does three things: pick the number, write it into the Port field, and
rewrite the token in the Command field.

Caption beneath the Port field, `aria-live="polite"`, one of:

- `Pinned :5176 and updated --port in the command. (5175 is in use right now.)`
- `Pinned :5174 — 5173 is pinned by IELTS Coach. Command updated.`
- `Pinned :5173 (Vite's default, free right now). Command updated.`
- `Couldn't find a free port near 5173 — enter one yourself.`

**Prefer the framework default when it is free.** Do not move for no reason.

**Verify**: `npm run typecheck` → 0.

### Step 3: The token rewrite — a pure module with tests

This is the part that can silently produce a broken command, so it goes in its
own module with **zero imports**, tested by `node --test` exactly as
`src/dragGeometry.ts` is. Read that file and its `.test.mjs` first and copy the
arrangement.

Export one function: given the current command string, a port, and a desired
form (`"--port"` | `"PORT="`), return the rewritten command.

Rules, each of which has a failure mode if broken:

- **Replace an existing token, never append a second one.** Recognise
  `--port N`, `--port=N`, `-p N`, `PORT=N`, `set PORT=N`.
- **Flag form by framework**: `Vite`/`Astro`/`Nuxt`/`SvelteKit`/`Angular`/`Remix`
  → `--port N`; `Next` → `-p N`; `CRA` → a `PORT=N ` prefix (react-scripts has no
  port flag); unknown → whichever button was pressed.
- **`npm` needs `--` before pass-through flags; `pnpm`/`yarn` do not.** Derive
  the package manager from **the command string's first token**, not from the
  dialog's `packageManager` state — that state is reset to `"npm"` on every
  dialog open and is unreliable here.
- **If ` -- ` is already present, append after it** — `npm run dev --workspace web -- --port 5176`,
  never a second `--`.
- **Anchor the `-p` pattern after the script name**, or it false-matches
  `mkdir -p tmp && npm run dev` and `pnpm -p` (which means `--parallel`).
- For `Vite` specifically, also append `--strictPort`. It is a real flag and it
  converts the 60-second-then-kill into a ~1 second honest `crashed` whenever
  the port is taken at bind time.

**Tests** (`node --test`), at minimum:

1. `npm run dev` + Vite + 5176 → `npm run dev -- --port 5176 --strictPort`
2. `npm run dev --workspace web -- --port 5175` + 5176 → the **existing** token is
   replaced and there is exactly one ` -- `
3. `pnpm dev` + Vite → no `--` inserted
4. `next dev` / `Next` → `-p 5176`
5. `mkdir -p tmp && npm run dev` → the `-p` in `mkdir -p` is **not** touched
6. `PORT=3000 npm start` + `PORT=` form → the existing `PORT=` is replaced, not duplicated
7. A command with no recognisable token → the token is appended in the right place

**Verify**: `node --test src/<your file>.test.mjs` → all pass.

### Step 4: Self-check

Report each:

- `grep -n "^import" src/<the pure module>.ts` → **no output**.
- `grep -rn "sniff_port_suggestion" src-tauri/src/registry.rs` → unchanged.
- `grep -rn "extra_env\|PORT" src-tauri/src/run.rs` → nothing new from you.
- `git status --short` → only in-scope files.

**Verify**: all three gates green, plus `node --test`.

## Test plan

Step 3's `node --test` cases are the machine-checkable part, and they are the
part that matters — the token rewrite is where this silently breaks.

Manual checks for the reviewer/maintainer:

- Edit `auto-job-applier web` (command already has `-- --port 5175`), press
  `Choose for me` → the port field and the existing `--port` token both change
  to the same new number, and there is still exactly one ` -- `.
- Edit IELTS Coach with 5173 free → it stays on 5173 and says so.
- With something else holding 5173 → it moves, and the caption names why.
- Press it with the Path field empty → the button is disabled.
- Save, then Run → the server binds the port Hangar is waiting on.

## Done criteria

- [ ] All three gates green plus `node --test`; report `cargo test` before/after
- [ ] The pure module has zero imports and ≥7 passing cases
- [ ] `find_free_port` returns `None` on exhaustion, never `from`
- [ ] `sniff_port_suggestion` unchanged; no `PORT` injected in `run.rs`
- [ ] `plans/README.md` status row for 043 updated

## STOP conditions

Stop and report back if:

- You are tempted to change what `sniff_port_suggestion` returns. Out of scope,
  and §10 step 4 explains why in writing.
- You are tempted to set `PORT` in the child environment. One of the
  maintainer's three projects ignores it, and the failure is a 60-second wait
  followed by killing a working server.
- The token rewrite cannot be made a zero-import module reachable by
  `node --test`. Report rather than adding a transpiler — `dragGeometry.ts`
  proves it is possible.
- Recognising an existing token reliably seems to need a real shell parser.
  Report; a wrong rewrite is worse than no button.

## Maintenance notes

- **The sentence to keep**: Hangar's `port` is a prediction, not an
  instruction. Any future feature that changes the port without changing the
  command re-introduces the 60-second-then-kill failure.
- The `--strictPort` addition for Vite is worth more than it looks: it turns the
  worst failure mode (silent bind elsewhere → timeout → kill) into a fast,
  honest crash with a readable reason.
- If a fourth package manager or framework appears, the token table is the only
  place to edit — that is why it is a pure module and not inline in the dialog.
