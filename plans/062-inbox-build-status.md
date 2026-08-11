# Plan 062: The inbox, rebuilt around what is actually in it

> **Supersedes plan 054.** Read that plan's status row before starting; it
> assumed an inbox of conversations, and the measurement below says otherwise.
>
> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise.
>
> **Drift check**: `grep -n "get_github_status" src-tauri/src/commands.rs && grep -n "InboxPanel" src/App.tsx && grep -n "\*\*Inbox\*\* (added" SPEC.md`
> All three must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo check
> --no-default-features --all-targets`, `cargo test` and `npm run typecheck`.
> Your reviewer runs `npm run build` and the bundle.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED — second network feature; the credential rules from §18 apply
  unchanged
- **Depends on**: 053 (merged — the token, the keychain, the client)
- **Category**: feature
- **Planned at**: 2026-08-11

## The measurement that reshaped this plan

Plan 053's maintenance note required one question be answered before slice 2
was built: *would the inbox be empty?* It was answered, against the live API,
on 2026-08-11:

| | |
|---|---|
| Unread notifications | **50** |
| `CheckSuite` (build results) | **50** |
| `Issue` | **0** |
| `PullRequest` | **0** |
| `Discussion` / mentions | **0** |
| Stars / forks / watchers, both repos | **0** |

So the answer is **both** things at once, and each changes the design:

1. **The conversation inbox is empty.** The original ask was *"questions from
   other developers that use the repository."* Nobody has found the repo — it
   went public an hour ago. Building a threaded reply UI now means building it
   blind, against zero real examples.
2. **Something else is very much there** — and it is a wall of *fifty identical
   rows*: `CI workflow run failed for main branch`, over and over.

**Listing those 50 notifications would be the worst possible feature.** Fifty
rows saying the same thing is not an inbox, it is noise with a scrollbar.

## What to build instead

**One line per project: is its build red or green right now?**

Not a notification list — a *state*. Fifty "CI failed" notifications collapse
to a single fact per repository, which is the only thing anyone acts on.

This is also a live finding, not a hypothetical: measured the same day, the
maintainer's second project's CI has been failing on every run and he did not
know. That is precisely the thing an app you open first thing should say.

## Scope

**In scope**:
- `src-tauri/src/github/client.rs` — one new read. Prefer
  `GET /repos/{owner}/{repo}/commits/{ref}/check-runs` or the combined status
  endpoint over `/notifications`; **say which you chose and why.** The
  notifications feed is the wrong shape for a state question and it mutates
  read/unread state, which this feature must not do.
- `src-tauri/src/github/` — repo detection from `.git/config`. **Read the file
  directly. Do NOT shell out to `git remote`** — `run_lookup` sets no `cwd`, so
  a bare `git remote -v` would resolve against Hangar's own directory and
  report every project as the same repository. That mistake was caught once
  already; do not reintroduce it.
- `src-tauri/src/commands.rs` — extend the existing GitHub command surface.
- `src/components/InboxPanel.tsx` — the list.
- `SPEC.md` §18 and §11's Inbox entry — amend to say what this actually shows.

**Out of scope** (do NOT build):
- **Threaded reading and replying.** That was slice 3 / plan 055, and there is
  nothing to read. Revisit when a real issue exists.
- **Marking notifications read**, or any write to GitHub. Hangar reads.
- Any change to `run.rs`, `process.rs`, §6, §8, §9, or the launch line.
- Any second HTTP client. Use `github/client.rs`'s existing `send()`.
- Any new dependency, any new Tauri plugin.
- Polling. Same rule as Ports and Doctor: read on open, and on Refresh.

## The §18 rules that still apply, unchanged

Re-read `github/secret.rs` before touching anything:

- **No `impl From<GithubError> for String`.** It still must not exist. That one
  line is what keeps a credential-bearing error out of a §7 toast.
- `Secret` has no `Display`; `expose()` gains **no** new call site.
- **Offline and rate-limited are `Ok` values, never `Err`.**
- Bounded twice — `reqwest`'s timeout and an outer `tokio::time::timeout`.
- **No retries.**
- Nothing runs before the grid renders, and nothing runs without a token.

## Design rules

- **A project not on GitHub, with no remote, or invisible to the token, is
  simply absent** — no row, no error, no toast. §11's Inbox entry already says
  this; honour it.
- **The unit is the repository, never the project.** Two cards sharing one repo
  root produce one row. §11 says this too.
- **Say when it was checked.** A stale green is worse than no green.
- **A check that could not run must never render as a passing build.** This is
  the third time this rule has come up in this repo — `vcs.rs` and the Doctor
  panel both state it. Unknown is its own state, visibly distinct from green.

## Steps

1. **Repo detection** from `.git/config`, pure and tested: HTTPS remotes, SSH
   remotes (`git@github.com:owner/repo.git`), a missing `origin`, a non-GitHub
   host, and a `.git` **file** (worktrees) rather than a directory. Each yields
   *no repo*, never an error.
2. **The client read**, following the existing `send()` shape exactly.
3. **Fold into the panel**, one row per distinct repository.
4. **Amend §18 and §11.** State plainly that the inbox shows build state, that
   conversations are deferred until there are any, and **why** — cite the
   measurement.

Verify after each: `cargo check --all-targets` → 0; `cargo check
--no-default-features --all-targets` → 0; `cargo test` → report before/after
(191 passed / 3 ignored today); `npx tsc --noEmit` → 0.

## Done criteria

- [ ] Four gates green; `cargo test` before/after reported
- [ ] `grep -rn "impl From<GithubError> for String" src-tauri/src` → **empty**
- [ ] `grep -rn "\.expose()" src-tauri/src` → still exactly the two production
      sites from 053. Paste it.
- [ ] `grep -rn "git remote\|\"remote\"" src-tauri/src/github` → **empty**
      (detection reads `.git/config`)
- [ ] A repo whose check could not run renders as **unknown**, not green —
      proved by a test, then mutation-tested
- [ ] No write to GitHub anywhere in the diff
- [ ] `capabilities/default.json` byte-unchanged
- [ ] `plans/README.md` updated, **and plan 054 marked SUPERSEDED by this one**

## STOP conditions

- You are about to build threaded reading or a reply box. That is deferred, on
  purpose, for lack of anything to read.
- You are about to call `/notifications` in a way that marks anything read.
- Detection needs to shell out to git. It does not — read `.git/config`.
- You need a new `expose()` call site. You do not.

## Maintenance notes

- **Revisit the deferral when the first real issue arrives.** The trigger is
  concrete: a non-zero `Issue` count in the notifications feed, or a stranger's
  comment. At that point plan 055 becomes worth writing, against a real example
  instead of an imagined one.
- The lesson worth keeping from this plan's own history: plan 054 was written
  from a vision, and one API call against the live account replaced it with
  something both smaller and more useful. **Measure the feed before designing
  the reader.**
