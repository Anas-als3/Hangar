# Plan 053: The GitHub credential — slice 1 of §18

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise. Update this plan's row in
> `plans/README.md` when done, unless a reviewer told you they maintain it.
>
> **Drift check**: `grep -n "## 18. GitHub integration" SPEC.md && grep -n "Inbox\*\* (added" SPEC.md`
> Both must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo test` and
> `npm run typecheck`. Your reviewer runs `npm run build` and the bundle.

## Status

- **Priority**: P2 — the maintainer ratified §18 knowing the cost
- **Effort**: M
- **Risk**: **HIGH** — this is the first credential this app has ever held, and
  the first network access. Every constraint below is load-bearing.
- **Depends on**: SPEC.md §18 (`28cec5d`), §11 inbox amendments (`880102f`)
- **Category**: feature
- **Planned at**: commit `880102f`, 2026-08-10

## Why this slice first

Slice 1's *payoff* is small — you connect a token and see "Connected as @you".
Its **value is that the credential mechanism is reviewable in isolation, before
one byte of repository data flows.** Say that plainly in your report rather
than overselling it.

It also unlocks a question that decides whether slice 2 is worth building at
all — see "The experiment" below.

**Read SPEC.md §18 in full before anything else.** It is short, it is mostly
constraints, and it was written for exactly this.

## Run this experiment BEFORE committing the keychain crate

`codesign -dv --verbose=4 /Applications/Hangar.app` reports an **ad-hoc**
signature and there is no `.entitlements` anywhere under `src-tauri/`. So:

- the data-protection keychain is unavailable — use the **file-based login
  keychain** and never set `kSecUseDataProtectionKeychain`;
- reads will raise the "Hangar wants to use your confidential information"
  panel;
- `CDHash` is a hash of the binary, so **"Always Allow" probably does not
  survive a rebuild** — and `npm run install:app` rebuilds on every change.

Store a dummy value, quit, relaunch, read it back. Rebuild, read again. Then
try under `npm run tauri dev`. **Write whatever actually happens into
`plans/README.md`'s "Environment facts" block before the crate is committed.**

This is not a blocker. But **a denied keychain must never render as "no
token"** — otherwise the maintainer watches his token vanish after every
rebuild and concludes the feature is broken. Distinguish *denied* from
*absent* in the status the UI shows.

## Dependencies — Cargo crates, not Tauri plugins

§4 pins **plugins** to exactly three. **Crates** are governed by the
one-line-justification rule, and `sha2` / `libc` / `win32job` already carry
exactly such comments in `Cargo.toml`. **No §4 amendment is needed and none may
be proposed.** `capabilities/default.json` stays untouched — all I/O is in Rust,
so the webview needs no new permission and the token never reaches it.

Add, each with its justification comment at the import:

- `keyring` — §18 requires the OS keychain with no disk fallback; std has none.
- `reqwest` (rustls, json; **no default features**) — §18 permits exactly one
  provider over TLS. Record the **binary size before and after** in your report.

## The `Secret` type — the reviewable invariant

The token must be structurally unable to reach a toast, a log line, an error
string or a panic.

- A newtype that **does not implement `Display` or `Debug`** — or implements
  `Debug` as a fixed redaction and nothing else.
- It is never a `String` in any signature that returns `Result<_, String>`.
- **No `impl From<GithubError> for String`.** Adding one later is a line a
  reviewer sees; without it, a `?` cannot silently convert an error carrying
  request context into a toast.
- Requests build the `Authorization` header at the call site from the secret
  and never store the composed header anywhere.

Write **guard tests** for these: a test that fails to compile is not available,
so assert what you can — that the redaction contains no token bytes, that error
types carry no secret field, that the cache struct has no token field.

## Scope

**In scope**:
- `src-tauri/src/github/` — new module: `mod.rs`, `secret.rs`, `keychain.rs`,
  `client.rs`, `error.rs`
- `src-tauri/src/commands.rs` — three new commands, registered in `main.rs`
- `src-tauri/Cargo.toml` — the two crates
- `src/types.ts`, `src/api.ts` — the mirrors
- `src/store.ts` — `inboxOpen` and the connection status
- `src/components/InboxPanel.tsx` — new; **shell only**: disconnected and
  connected states, no list, no threads
- `src/App.tsx` — the header **Inbox** button, mount the panel, add `inboxOpen`
  to the `inert` overlay set
- `src/components/ProjectGrid.tsx` — the folder band's Esc guard

**Out of scope** (do NOT build):
- **Any notification fetch, any repository detection, any cache.** Those are
  slices 2 and 3. If you write a `.git/config` parser you have exceeded scope.
- Any change to `run.rs`, `process.rs`, the §6 state machine, §8 or §9.
- Any §7 change beyond **adding** commands. No rename, no reshape.
- Markdown rendering, `dangerouslySetInnerHTML` anywhere.
- Any retry logic.
- Any new Tauri plugin.

## The rules that cannot bend

- **No GitHub code may be reachable from `main.rs`'s setup, `run.rs`, or the
  startup path before the grid renders.** The token is read **lazily, once per
  session, on first GitHub use**.
- `GithubState` is its own managed cell. It must **not** live inside
  `AppState.projects` / `.runtime` / `.dev_env`, and no GitHub call may hold any
  lock those use.
- Validation happens **before** storage: `GET /user` proves auth; the response's
  scope headers are read when present.
- **Offline/rate-limited are `Ok` values, not `Err`** — §7 turns every `Err`
  into a toast, and §18 makes offline a first-class state.
- Every request bounded **twice**: `reqwest`'s own timeout *and* an outer
  `tokio::time::timeout` in the one private `send()`. The outer one is what a
  test can assert.
- **No retries anywhere.**

## Steps

1. **The experiment** (above). Record it in `plans/README.md`'s Environment
   facts. **Do this first.**
2. `secret.rs` + `error.rs` + the guard tests. Verify: `cargo test`.
3. `keychain.rs` — store / read / delete, distinguishing *denied* from *absent*.
4. `client.rs` — the one `send()` with double timeouts, and `validate()`.
5. The three commands: `set_github_token`, `get_github_status`,
   `remove_github_token`. Register in `main.rs`. Mirror in `types.ts`/`api.ts`.
   Feed the wire-drift guard a fully-populated sample — every `Option` `Some`,
   or `skip_serializing_if` hides the keys and the guard passes vacuously.
6. The panel shell + header button + the two Esc sites.

Verify after each: `cargo check --all-targets` → 0; `cargo test` → all pass
(report before/after); `npm run typecheck` → 0.

## The six failure messages — ratify them here so slice 2 does not invent them

- **bad token** — 401 on `/user`.
- **expired or revoked** — 401 with a stored token; offer **Reconnect**.
- **insufficient scope** — 403 **and** `x-ratelimit-remaining` ≠ 0.
- **rate-limited** — 403/429 with `remaining: 0`; reset from
  `x-ratelimit-reset`. **429 with `retry-after` is a secondary limit and gets
  its own wording** — telling someone to wait an hour when it is 60 s is its
  own bug.
- **offline**.
- **keychain refused** — never "no token".

**Get the 403 split right.** It hinges on `x-ratelimit-remaining`; inverting it
swaps the two most confusing messages in the feature.

## Two things to state plainly in the UI

- **Scopes**: classic PAT, `notifications` + `repo`. Say in the panel and the
  README that `repo` is broad — full read/write to code — and that
  `public_repo` suffices if every repository is public. A developer deciding
  whether to trust this app deserves to read that rather than discover it.
- **You must be online once to connect.** §18 makes offline first-class for the
  *inbox*, not for setup. That is within the letter, but it is a real behaviour.

## Done criteria

- [ ] Three gates green; `cargo test` before/after reported
- [ ] The keychain experiment was run and its result written into the index
- [ ] `Secret` has no `Display`; no `From<GithubError> for String`
- [ ] No GitHub code reachable from `main.rs` setup, `run.rs`, or startup
- [ ] Binary size before/after recorded
- [ ] `capabilities/default.json` untouched; no new Tauri plugin
- [ ] `plans/README.md` row updated

## STOP conditions

- The keychain cannot be reached at all from an ad-hoc-signed app. Report the
  experiment's result — do **not** fall back to a file. §18 forbids it.
- You find yourself writing repository detection or a notifications fetch.
  That is slice 2.
- Any GitHub call needs a lock `run.rs` or `process.rs` uses.
- `reqwest` pulls in OpenSSL rather than rustls. Report the feature flags.

## Maintenance notes

- The whole feature's safety rests on `Secret` and on the absence of a
  `From<GithubError> for String`. Both are one-line deletions away from being
  useless — check them first in any future review.
- Slice 2 must confront a real risk before it is built: all three of the
  maintainer's repositories are solo-owned, and **GitHub does not notify you
  about your own activity**. With slice 1's token in place, one throwaway
  command printing the row *count* per repo answers in thirty seconds whether
  the inbox would be empty. If it is, take that to the maintainer rather than
  shipping an empty panel.
