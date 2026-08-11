# Plan 036: Stop losing builds to a manual copy, and widen the external-API group

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `grep -n "\"openai\"" src-tauri/src/registry.rs && grep -n "build:app" package.json`
> Both must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. **Your reviewer runs `npm run build` and the bundle.**

## Status

- **Priority**: P1 for step 1 — three separate features have now been reported
  "not working" when they were merged, tested and simply not installed
- **Effort**: S
- **Risk**: LOW — one npm script, one README section, one data table
- **Depends on**: plan 035 (DONE, merged `55d318a`)
- **Category**: dx + feature
- **Planned at**: 2026-08-10

## Why this matters

### Step 1 — the recurring failure

`npm run build:app` writes to
`src-tauri/target/release/bundle/macos/Hangar.app`. The maintainer runs
`/Applications/Hangar.app`, which is a **hand-copied snapshot**. Building does
not update it, and nothing says so.

This has now produced three false bug reports in one day:

| Reported | Actual cause |
|---|---|
| "i opened the app it doesnt show" | stale `/Applications` copy |
| (folders/drag/libraries invisible) | stale `/Applications` copy |
| "the stack of auto applier doesnt show" | installed binary 10:20, built binary 13:08 |

Each cost a diagnosis cycle for a defect that did not exist. The fix is a
script, not a habit.

There is a second, subtler symptom: **the maintainer's own bug reports are
unreliable evidence** while this is possible, because "it doesn't work" and "I'm
looking at yesterday's build" are indistinguishable from the outside.

### Step 2 — external APIs

The maintainer's words:

> if possible to see what external apis each project has

Plan 035 added `openai` → `OpenAI` and `@anthropic-ai/sdk` → `Anthropic` at the
head of `LIBRARY_ALLOW_LIST`, and the list already carried `@supabase/supabase-js`,
`firebase`, `@apollo/client`, `graphql`, `@trpc/client` and `axios`. That is a
thin sample of the third-party services a real project talks to.

This step widens **only** the head group — the services and APIs a project calls
out to. It changes no shape, no schema, no command.

## Current state

`package.json` scripts today:

```json
"build:app": "tauri build",
```

`src-tauri/src/registry.rs` — `LIBRARY_ALLOW_LIST`'s head group, added by plan 035:

```rust
("openai",            "OpenAI"),
("@anthropic-ai/sdk", "Anthropic"),
```

followed by the original 19 entries, then plan 035's testing/automation tail.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust check | `PATH="$HOME/.cargo/bin:$PATH" cargo check --manifest-path src-tauri/Cargo.toml --all-targets` | exit 0 |
| Rust tests | `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml` | all pass (baseline 124 — **run it first and report what you observe**) |
| TypeScript | `npm run typecheck` | exit 0 |

**Do not run** `npm run build`, `npm run verify`, `npm run build:app`, the new
`install:app` script, or `npm run test:acceptance`. Several take minutes with no
output and a 600 s watchdog has killed executor runs here. **In particular, do
not execute the install script you are writing** — it deletes and replaces an
application bundle on the maintainer's machine.

## Scope

**In scope**:
- `package.json` — one new script
- `README.md` — a short section documenting it
- `src-tauri/src/registry.rs` — the head group of `LIBRARY_ALLOW_LIST`, and tests

**Out of scope** (do NOT touch):
- `detect_stack`, `read_workspace_members`, `declared_workspace_dirs`,
  `has_dependency`, `sniff_port_suggestion`, `FRAMEWORK_DETECTORS` — plan 035
  just landed all of them and they are correct.
- The existing 19 library entries and plan 035's testing tail. **Their relative
  order must not change** — that is what keeps existing test expectations valid.
- `ProjectStack`'s shape, `PackageJsonInfo`'s shape, any §7 command, `types.ts`.
  This adds no wire key.
- `src/` entirely. No UI change in this plan.
- Anything in `src-tauri/src/run.rs` or `commands.rs`.
- Code signing, notarisation, `.dmg` handling — plan 024 owns distribution and
  it is a maintainer decision.

## Git workflow

- One commit per step: `Install script: <what>` / `Stack: <what>`.

## Steps

### Step 1: `npm run install:app`

Add one script to `package.json` that builds **and** installs:

```json
"install:app": "npm run build:app && rm -rf /Applications/Hangar.app && cp -R src-tauri/target/release/bundle/macos/Hangar.app /Applications/"
```

Requirements and constraints:

- It must **fail loudly** if the build fails — `&&` chaining, never `;`. A
  half-run script that deletes the installed app and then fails to copy would
  leave the maintainer with no Hangar at all. This is the single most important
  property of this step.
- Do **not** add `open /Applications/Hangar.app` to the script. Launching is the
  maintainer's decision, and a script that opens a GUI app is surprising in CI.
- Do **not** try to detect or kill a running Hangar. Replacing a running bundle
  is the maintainer's call and killing their app would orphan the dev servers it
  supervises — the exact failure §8 exists to prevent. The README covers it
  instead.
- macOS-only path. Add a one-line comment in the README saying so; do not build
  cross-platform install logic for a project that ships no Windows build (plan
  024).

**Verify**: `npm run typecheck` → exit 0 (proves `package.json` still parses).
Do **not** execute the script.

### Step 2: Document it where a human will look

Add a short section to `README.md`, near the existing build instructions:

- The one command.
- **Quit Hangar first with Cmd+Q, not Force Quit** — Cmd+Q runs the §9 quit path
  that stops running dev servers cleanly; a kill orphans them.
- One sentence naming the trap plainly: `npm run build:app` writes to
  `src-tauri/target/release/bundle/`, and `/Applications/Hangar.app` is a copy —
  building alone changes nothing you can see.

**Verify**: the section exists and names both the command and the Cmd+Q step.

### Step 3: Widen the external-API / service group

In `LIBRARY_ALLOW_LIST`, extend the **head** group only. Keep `openai` and
`@anthropic-ai/sdk` first; everything already there keeps its relative order.

Add, grouped by what they are — each entry is a service a project **calls out
to**, which is the whole selection rule:

- **AI**: `@google/generative-ai` → `Gemini`, `@mistralai/mistralai` → `Mistral`,
  `cohere-ai` → `Cohere`, `replicate` → `Replicate`, `@huggingface/inference` →
  `HuggingFace`, `langchain` → `LangChain`, `ai` → `Vercel AI`
- **Payments / comms**: `stripe` → `Stripe`, `twilio` → `Twilio`,
  `@sendgrid/mail` → `SendGrid`, `resend` → `Resend`, `nodemailer` → `Nodemailer`
- **Cloud / platform**: `@aws-sdk/client-s3` → `AWS`, `aws-sdk` → `AWS`,
  `googleapis` → `Google APIs`, `@octokit/rest` → `GitHub API`,
  `@vercel/blob` → `Vercel Blob`
- **Auth**: `@clerk/clerk-sdk-node` → `Clerk`, `@clerk/nextjs` → `Clerk`,
  `next-auth` → `NextAuth`, `@auth0/auth0-react` → `Auth0`
- **Observability**: `@sentry/node` → `Sentry`, `@sentry/react` → `Sentry`,
  `posthog-js` → `PostHog`
- **Data stores** (they are external services too): `mongodb` → `MongoDB`,
  `mongoose` → `MongoDB`, `pg` → `Postgres`, `mysql2` → `MySQL`,
  `redis` → `Redis`, `ioredis` → `Redis`

`detect_stack` dedupes by display name, so the paired keys (`aws-sdk` /
`@aws-sdk/client-s3`, both Sentry keys, both Clerk keys, `mongodb`/`mongoose`,
`redis`/`ioredis`) collapse to one entry each. Confirm that in your report.

**Do not** add: HTTP clients beyond the `axios` already present (`node-fetch`,
`got`, `undici` — plumbing, not a service), test doubles (`msw`, `nock`), or
anything that is a local library rather than a remote service. The selection rule
is "this project talks to something outside itself", and it must stay legible.

**Verify**: `cargo check --all-targets` → exit 0; `cargo test` → all pass with
**zero expectation changes**. Report any test you had to touch — you should have
to touch none, because the additions are inside the existing head group and the
19 originals keep their relative order.

### Step 4: Tests

Add to `registry.rs`'s test module, in the style of plan 035's:

1. A `package.json` with `stripe`, `@sentry/node` and `mongoose` yields
   `Stripe`, `Sentry`, `MongoDB`.
2. A paired-key dedupe test: `aws-sdk` **and** `@aws-sdk/client-s3` together
   yield exactly one `AWS`.
3. The head group leads: a project with `react` **and** `stripe` lists `Stripe`
   before `React`, so the card's visible three favour services over frameworks.

**Verify**: `cargo test` → all pass; report the new total.

### Step 5: Gates and self-check

Report each:

- `grep -n "install:app" package.json` → present, and uses `&&` not `;`.
- `grep -c "install:app" README.md` → at least 1.
- `git diff --stat` shows nothing under `src/`, `run.rs` or `commands.rs`.

**Verify**: `cargo check --all-targets` → 0; `cargo test` → all pass;
`npm run typecheck` → 0.

## Test plan

Steps 4's unit tests are the machine-checkable part. Manual checks for the
reviewer/maintainer:

- `npm run install:app` after quitting Hangar → the app in `/Applications` has
  today's timestamp and shows the new behaviour.
- Deliberately break the build (a syntax error), run the script → it stops
  **before** deleting `/Applications/Hangar.app`. This is the property that
  matters; verify it rather than assuming it.
- Run `example-monorepo server` once → its card lists `OpenAI · Anthropic ·
  Playwright +N` or similar, with the services first.

## Done criteria

- [ ] `cargo check --all-targets` 0; `cargo test` passes (report before/after);
      `npm run typecheck` 0
- [ ] `install:app` exists, chains with `&&`, and does not launch or kill anything
- [ ] README documents the command and the Cmd+Q step
- [ ] Zero existing test expectations changed
- [ ] No `src/` change, no `run.rs`/`commands.rs` change, no new wire key, no new
      dependency
- [ ] `plans/README.md` status row for 036 updated

## STOP conditions

Stop and report back if:

- The install script seems to need `sudo`, a code-signing step, or anything from
  plan 024. It does not — this is a local copy into `/Applications`, which the
  maintainer already does by hand.
- You are tempted to have the script kill or relaunch Hangar. Do not. Killing it
  orphans the dev servers it supervises.
- Widening the head group forces an existing test expectation to change. It
  should not; report rather than editing the expectation.
- The list starts to look like "every npm package the maintainer might use". The
  rule is **external services this project calls out to** — if you cannot say
  what remote endpoint an entry implies, it does not belong.

## Maintenance notes

- **The lesson worth keeping**: for three separate features today, "it doesn't
  work" meant "the build is not installed". Any future bug report about
  user-visible behaviour should start by comparing the timestamps of
  `/Applications/Hangar.app/Contents/MacOS/hangar` and the one under
  `src-tauri/target/release/bundle/`. That check takes five seconds and would
  have saved three diagnosis cycles.
- Future additions to the head group follow the same rule as the rest of the
  list (plan 023): **name a registered project that came up empty or
  misleading** — not "this service is popular".
- If the head group ever grows past roughly forty entries, that is the signal to
  revisit whether `libraries` should be split into categories on the wire, which
  is a §5 **and** §7 amendment and was deliberately deferred by plan 035.
