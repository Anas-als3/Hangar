# Plan 060: The launch line — what needs your attention, on open

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise. Update this plan's row in
> `plans/README.md` when done, unless a reviewer told you they maintain it.
>
> **Drift check**: `grep -n "fn build_report" src-tauri/src/preflight.rs && grep -n "ResumeLine" src/App.tsx && grep -n "\*\*Doctor\*\* (added" SPEC.md`
> All three must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo check
> --no-default-features --all-targets`, `cargo test` and `npm run typecheck`.
> Your reviewer runs `npm run build` and the bundle.

## Status

- **Priority**: **P1**
- **Effort**: M
- **Risk**: MED — one new §11 element, and it must not become a git client (§3)
- **Depends on**: 057 (merged) for the finding shape
- **Category**: feature
- **Planned at**: 2026-08-11, after 058 merged

## The measurement that justifies this

Run on the maintainer's real machine, 2026-08-11, one `git ls-remote` per
project:

| Project | State |
|---|---|
| `example-app` | in sync, clean |
| `Hangar` | in sync, 1 deliberate local edit |
| **`example-monorepo`** | **30 commits unpushed, local-only for 7 days** |

Thirty commits existing on exactly one laptop, unnoticed for a week. **Nothing
in the current UI says so, and nothing would ever have said so.** That is the
feature: not a status decoration, but the one thing worth interrupting someone
with when they sit down.

This entry was originally written *against* itself — the register guessed the
brief would read "nothing changed" every morning. Measuring overturned that.
Keep measuring before believing this plan too.

## Where it goes, and why not on the cards

§11's card element list is fixed and deliberately short. Adding a git line to
every card would put a permanent element on cards that have nothing to say —
the exact noise §11 spends three paragraphs preventing.

Instead: **one line under the header, rendered only when there is something to
report.** Silent otherwise — zero pixels, no empty state, no "all clear" badge.
It sits as a sibling to the existing resume line (see `ResumeLine` in
`src/App.tsx` and follow its shape).

- At most **three** items inline; beyond that, `+N` opens the Doctor panel.
- Each item is one clause: `example-monorepo · 30 unpushed`.
- Clicking an item scrolls its card into view. **That is the only action.**

## The §3 line this must not cross

§3's OUT list is absolute. **This reports; it never acts.**

- No push. No pull. No fetch. No commit. No stash. Not behind a confirm, not in
  a menu, not "just for convenience".
- The one permitted action is scrolling a card into view.
- If you find yourself adding a "Push" button, **STOP** — that is a git client,
  and §3 says Hangar is not one.

## Network: read this before writing the git code

`git ls-remote` — which is how the measurement above was taken — **is a network
call.** Doing one per project on launch would put the network on the startup
path, where a hung DNS lookup becomes a hung app. **Do not do it.**

Use the **local remote-tracking ref** instead: `origin/main` as of the last
fetch. No network, instant, no failure mode.

That trade has a consequence you must state honestly in the UI:

- **"Ahead" (unpushed) is accurate** — your own HEAD and your own tracking ref
  are both local facts. This is the case that matters, and the case that found
  the 30 commits.
- **"Behind" is only as fresh as the last fetch**, and Hangar will not fetch.
  So **do not report "behind" at all** rather than reporting a stale number as
  if it were current. A wrong "you're up to date" is worse than silence.

Report exactly three things, all local:

1. **Unpushed commits** — `rev-list --count <upstream>..HEAD`. No upstream
   configured → report nothing (not an error).
2. **Uncommitted changes** — a count, from a porcelain status. **Never file
   contents, never a diff.**
3. **Crashed last run** — already in the data model; surface it here too so one
   line answers "what needs me".

## Scope

**In scope**:
- `src-tauri/src/preflight.rs` or a sibling `src-tauri/src/vcs.rs` — the git
  reads, as pure functions over command output wherever possible. Say which and
  why.
- `src-tauri/src/commands.rs` — extend `get_preflight`'s report, **or** one new
  command if the shapes genuinely do not fit. Prefer extending.
- `src/components/` — the line component; model it on `ResumeLine`.
- `src/App.tsx` — mount it.
- `SPEC.md` §11 — the new element.

**Out of scope**:
- Any git *write*. See above.
- `run.rs`, `process.rs`, the §6 machine, §8, §9.
- Any change to the card element list. The cards do not change.
- Any network call of any kind.
- Branch switching, history browsing, diff viewing, blame. All §3.

## How to run git

**Use the existing §8 spawn helper.** Do not add a second way to run a
subprocess — CLAUDE.md names "one spawn helper" as a top-priority rule, and
`run.rs` already shells out to git for the pull step. Read how it does that and
match it, including the non-interactive environment (a git that prompts for
credentials on a startup path would hang the line forever).

Every git call needs a **timeout** and must treat failure as "report nothing
for this project", never as an error toast.

## Steps

1. **The git reads**, with tests over captured output. Cover: no upstream
   configured; a detached HEAD; a path that is not a git repo at all; a repo
   with zero commits. Each yields *nothing*, not an error.
2. **Wire it into the report**, with the wire-drift guard fed a fully-populated
   sample — every `Option` `Some`, every vector non-empty.
3. **The line component.** Silent when empty. Three items max, then `+N`.
4. **§11 amendment**, one paragraph, stating: silent when there is nothing;
   at most three items; the only action is scroll-into-view; **it never pushes,
   pulls or fetches**; and that "behind" is deliberately not reported because
   Hangar does not fetch.

Verify after each: `cargo check --all-targets` → 0; `cargo check
--no-default-features --all-targets` → 0 (this keeps the Windows gate alive);
`cargo test` → report before/after; `npx tsc --noEmit` → 0.

## Done criteria

- [ ] Four gates green; `cargo test` before/after reported
- [ ] **`git grep -nE "\"(push|pull|fetch|commit|stash)\"" src-tauri/src` shows
      no new call site.** Paste the output.
- [ ] No network call in the diff
- [ ] The line renders nothing when every project is clean — proved by a test
- [ ] "Behind" is not reported anywhere
- [ ] `plans/README.md` row updated

## STOP conditions

- You are about to run `git fetch` or `git ls-remote`. Both are network calls on
  a startup path. Stop.
- You are about to add a push/pull control. That is §3's line.
- The report would need a second spawn helper. It does not — use the §8 one.
- Reading uncommitted *content* rather than counting files. Only counts leave
  this module.

## Maintenance notes

- The pressure this feature will attract is "I can see it's unpushed, let me
  add a Push button." That single button turns a reporting tool into a git
  client with a rounded-corner UI, and it is the thing to reject in review.
- The honesty to preserve: **Hangar never fetches, so it can never tell you the
  remote moved.** If that sentence ever stops being true, this plan's whole
  network argument needs rewriting rather than quietly extending.
