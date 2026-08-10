# Plan 058: Get §8's Windows code back under a compiler

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise.
>
> **Drift check**: `grep -n "^reqwest" src-tauri/Cargo.toml && grep -n "^mod github" src-tauri/src/main.rs`
> Both must match. On a mismatch, STOP.
>
> **Gate ownership**: you run every command in this plan. Your reviewer re-runs
> them and additionally runs `npm run build`.

## Status

- **Priority**: **P1**, but read "The real fix is not this plan" first
- **Effort**: S–M
- **Risk**: MED — touches `main.rs`'s command registration, which is §7 surface
- **Depends on**: nothing
- **Category**: tech debt / verification
- **Planned at**: commit after the 013 merge, 2026-08-11

## The real fix is not this plan

**CI has never run — 60 of 60 runs failed, every one without being assigned a
runner.** That is a billing condition on a private repo with 10×-billed macOS
runners, and only the maintainer can resolve it. If he does, `ci.yml`'s
`windows-latest` job compiles *and runs* the `#[cfg(windows)]` code on a real
Windows machine where `windows.h` exists, `aws-lc-sys` builds, and this whole
problem evaporates.

**Say that in your report.** This plan is the fallback that gives fast local
feedback on a laptop; it is not a substitute for running the code on Windows,
and nothing here should be described as if it were. A `cargo check` proves the
Windows code *compiles*. It does not prove `TerminateJobObject` kills anything.

## What is actually broken

Before §18 slice 1, this passed on this machine:

```
PATH="/opt/homebrew/opt/llvm/bin:$PATH" \
  cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc
```

Verified by bisect at `2f68088` — the commit before `reqwest` entered
`Cargo.toml`. It now dies in `aws-lc-sys`'s C build script for want of
`windows.h`. `ring` fails the same way (`assert.h`), so this is not about which
crypto provider rustls uses; it is the absent Windows CRT/SDK on macOS.

The `llvm-rc` part of that PATH is already recorded in the index's Environment
facts and is **not** optional — without it you get a `tauri-winres` panic
(`NotAttempted("llvm-rc")`) that looks like a different bug entirely.

## The approach

Put the GitHub module behind a **default-on** Cargo feature, so the Windows
cross-check can be run with `--no-default-features` and reach Hangar's own code.

```toml
[features]
default = ["github"]
github = ["dep:reqwest", "dep:keyring"]
```

Nothing about the shipped app changes: `default` includes `github`, so every
existing build, test and bundle behaves exactly as it does today.

## Scope

**In scope**: `src-tauri/Cargo.toml`, `src-tauri/src/main.rs`,
`src-tauri/src/commands.rs` (the `cfg` attributes only), a short section in
`README.md` or the index recording the command, `.github/workflows/ci.yml` (one
added step — see step 4).

**Out of scope**:
- **Any behaviour change when the feature is on.** This is a compile-time
  reorganisation. If `cargo test` output changes by even one test, you have
  gone too far.
- Any change to `github/`'s internals, `Secret`, the keychain, or the client.
- Any change to `run.rs`, `process.rs`, the §6 machine, §8, §9.
- Removing, renaming or reshaping any §7 command. See the §7 note below.
- Installing `xwin` or any Windows SDK. **Do not download a Microsoft SDK** —
  that carries a licence the maintainer has not accepted.

## The §7 question — resolve it this way

§7 is FROZEN, and three GitHub commands are registered in `main.rs`. Making
them conditional means a `--no-default-features` build exposes fewer commands
than §7 lists.

**That is acceptable, and here is the reasoning to put in the code comment**:
§7 freezes the *shape* of the API — names, arguments, payloads — and CLAUDE.md
explicitly permits implementing **subsets**. A build configuration that omits a
command is a subset, exactly like M1 was. What §7 forbids is a command that
exists under a different name or a different shape, and nothing here does that.

**The shipped binary always has all three**, because `default = ["github"]`.

## Steps

### Step 1: The feature

Move `reqwest` and `keyring` to optional, add the `[features]` block. Keep
their existing justification comments — they are still §4-required.

**Verify**: `cargo check --manifest-path src-tauri/Cargo.toml --all-targets` → 0,
unchanged from today.

### Step 2: The cfg attributes

`#[cfg(feature = "github")]` on: `mod github;`, the `app.manage(...)` line, the
three `commands::…` registrations, and the command functions themselves.

If the frontend's `api.ts` calls a command that a `--no-default-features` build
lacks, **that is fine and needs no TS change** — the default build has them and
that is the only build ever shipped. Do not add TS conditionals.

**Verify**:
- `cargo check --all-targets` → 0
- `cargo check --no-default-features --all-targets` → 0
- `cargo test` → **exactly the same count as before your change** (currently
  168 passed / 3 ignored). A different number means the feature is gating a
  test it should not.

### Step 3: The cross-check, restored

```
PATH="/opt/homebrew/opt/llvm/bin:$PATH" \
  cargo check --manifest-path src-tauri/Cargo.toml \
  --target x86_64-pc-windows-msvc --no-default-features --all-targets
```

**This must reach and compile Hangar's own code.** Confirm with
`2>&1 | grep -c "Compiling hangar"` → 1, not 0. A run that fails in a
dependency's build script before reaching `hangar` has proved nothing, and that
is exactly the trap this plan exists to escape.

Report the full output. If §8's `#[cfg(windows)]` code has *genuine* compile
errors that have been hiding since the cross-check broke, **that is the point
of this plan** — report them, fix nothing, and let the reviewer decide. Do not
quietly patch Windows logic under a plan about build configuration.

### Step 4: Make it a real gate

Add one step to `ci.yml`'s existing macOS `host` job:

```yaml
- name: Windows cross-check (compile only, no TLS stack)
  run: |
    rustup target add x86_64-pc-windows-msvc
    cargo check --manifest-path src-tauri/Cargo.toml \
      --target x86_64-pc-windows-msvc --no-default-features --all-targets
```

It belongs on the **macOS** job specifically: the `windows-latest` job builds
everything on real Windows and does not need this.

**Write the honest comment above it**: this checks that the Windows code
compiles; it does not run it, and it deliberately excludes the TLS stack, which
only the real `windows` job covers.

**Verify**: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml'))"`
parses. (If `yaml` is unavailable, say so — do not install it.)

## Done criteria

- [ ] `cargo check --all-targets` → 0; `cargo test` count **identical** to before
- [ ] `cargo check --no-default-features --all-targets` → 0
- [ ] The Windows cross-check **reaches `Compiling hangar`** — report the grep count
- [ ] `npx tsc --noEmit` → 0, with no TS change at all
- [ ] The shipped default build still registers all three GitHub commands
- [ ] Your report states plainly that this does not replace running the code on Windows

## STOP conditions

- `cargo test`'s count changes. The feature is gating something it should not.
- The cross-check still cannot reach `Compiling hangar` after the feature is in.
  Then some other C-building crate is in the default-off path too — **report
  which one**; do not start disabling things to chase it.
- You are tempted to install `xwin` or download a Windows SDK. Stop — that is a
  licence decision, not yours.
- §8's Windows code turns out not to compile. Report it. That finding is worth
  more than this plan.

## Maintenance notes

- The failure mode to watch: someone adds a crate with a C build script to the
  default-on set and this gate silently reverts to failing in a dependency. The
  `grep -c "Compiling hangar"` check in step 3 is what catches that — keep it
  in CI, not just in this plan.
- If CI billing is ever fixed, this gate stays useful anyway: it is seconds on
  a laptop versus minutes on a runner. But its description must never drift
  from "compiles" to "works".
