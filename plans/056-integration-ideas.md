# 056: Integration ideas — the register

Not a build plan. This is the ranked list the maintainer asked for, with the
evidence behind each one, so that whoever picks one up is not starting from a
blank page. Each entry says what it costs, what it needs, and — the part most
idea lists skip — **why it might not be worth building.**

Written 2026-08-11, against `f9b3fba`.

## The question that ranks these

Hangar's current value is "run this project." **A terminal already does that.**
So the ranking test is not "is this a nice feature" — it is:

> Does this tell the maintainer something a terminal cannot, at the moment he
> would otherwise have to go find it?

Anything that passes that test earns the app the right to be the first thing
opened in the morning. Anything that fails it is a second copy of `npm run dev`
with rounded corners.

## Evidence gathered on the real machine (2026-08-11)

These are measurements, not assumptions. Re-derive before trusting them later.

| Fact | Value |
|---|---|
| Project cards | 3 (`Example App`, `example-monorepo web`, `example-monorepo server`) |
| Distinct repos | 2, both `github.com/Anas-als3/…`, **both solo-owned** |
| `example-app` | `.nvmrc` 24.18.0, `engines` set, 12 direct deps, 205-line README, **no LICENSE**, no `.env.example` |
| `example-monorepo` | **no `.nvmrc`, no `engines`**, `.env` + `.env.example`, 26-line README, no LICENSE |
| **`.env` drift, real** | `.env.example` documents `ANTHROPIC_API_KEY`; the actual `.env` **does not contain it** |
| OSV batch query | 207 lockfile packages → 1 HTTP request → **0 advisories**, no key, no rate limit |
| Node in use | v24.18.0, matches `example-app`'s pin |

The `.env` row is the important one. It was found in about a second, by
comparing key *names* only. If that project reads `ANTHROPIC_API_KEY` at
runtime, it is already broken in a way that will present as a mystery crash —
which is exactly the failure the maintainer hit this morning from a different
cause and spent real time on.

---

## 1. Preflight — "will this even start?"

**Ranked first because it is the only idea with a confirmed hit on his own
machine before it was built.**

Before Run, check the things that make a start fail for reasons the logs
explain badly, and show them on the card:

- `.env` vs `.env.example` — keys documented but absent. **Names only, never
  values, never rendered.**
- `.nvmrc` / `engines` vs the Node that would actually be used.
- lockfile newer than `node_modules` → install needed. (`lastLockfileHash`
  already exists in the data model; this is the same idea, surfaced earlier.)
- the port already held by a foreign process — **already built**, this folds in.

**Cost**: S–M. **Needs**: nothing. No network, no auth, no new dependency.
**Why it might not be worth it**: if the checks are noisy or wrong they become
a permanent yellow badge the user learns to ignore, which is worse than no
check. Every check must be individually dismissable and silent when it passes.

## 2. The morning brief — what changed while you were away

The feature that actually answers "why open this first." On launch, per card,
what moved since the last run: commits behind/ahead of the remote, whether the
lockfile changed, how long since it last ran successfully, whether it crashed
last time and why (**the crash reason line already exists**).

**Cost**: M. **Needs**: git only — local, no auth.

**Measured 2026-08-11, and it overturns this entry's own objection.** The
original text here guessed the brief would read "nothing changed" every morning
with only three cards. Then it was actually measured:

| Project | State |
|---|---|
| `example-app` | in sync, clean |
| `Hangar` | in sync, 1 uncommitted (a deliberate local edit) |
| **`example-monorepo`** | **30 commits unpushed, sitting locally for 7 days** |

Thirty commits existing on exactly one laptop is a real risk to real work, it
had gone unnoticed for a week, and **nothing in the current UI would ever say
so.** That is not a "nice status line" — it is the single most valuable thing
this app could tell its owner on a Monday morning, and it was found by one
`git ls-remote` per project with no network write and no auth.

**Revised objection, which is smaller**: the brief must not become a second
git client (§3). It reports and links out; it never pushes, pulls or commits.

## 3. Dependency health via OSV.dev

**Verified working today.** `POST https://api.osv.dev/v1/querybatch` — Google's
vulnerability database, **no API key, no registration, no rate limit**, one
request for an entire lockfile. 207 packages answered in a single call.

The zero-auth part is what makes it rank above the GitHub inbox: it needs
nothing from the user, so it can be on by default.

**Cost**: S. **Needs**: network, no auth, no new Rust crate (`reqwest` landed
with §18 slice 1).
**Why it might not be worth it**: today the honest result is **0 advisories** —
both projects are clean. A feature whose current output is "everything is fine"
is easy to over-sell. Its value is that this changes without you noticing, and
the check is nearly free. Say that plainly in the UI rather than dressing up an
empty result as a security dashboard.

## 4. The ship checklist — his own idea, made automatic

The maintainer proposed "a to-do plan in the program that each project must
reach so it can be shipped." The upgrade: **most of it checks itself.** README
over N lines, LICENSE present, no CVEs (#3), `.env.example` complete (#1),
tests pass, last deploy green. Manual items sit alongside.

Both repos are missing a LICENSE right now — two items already tickable.

**Cost**: M. **Needs**: #1 and #3 first, or the auto-checks have nothing to
read.
**Why it might not be worth it**: a checklist that is mostly manual is a to-do
app, and there are better to-do apps. It is only worth building if the majority
of items check themselves — that ratio is the go/no-go.

## 5. Deploy status (Vercel / Netlify / GitHub Actions)

`GET https://api.vercel.com/v6/deployments` with `Authorization: Bearer`.
Actions is free with the token §18 already stores.

**Cost**: M. **Needs**: another token for Vercel; Actions needs none beyond §18.
**Why it might not be worth it — and this one is serious**: it is unknown
whether either project is deployed anywhere at all. **Ask before building.**
Building a deploy panel for projects that are never deployed is the most
expensive way to learn that. Actions-only is the cheap half — it reuses the
existing token and answers "did my last push go green."

---

## What this list changes about §18 slice 2

Slice 2 (the GitHub notification inbox) was next in the queue. This register
makes a case for **reordering, not cancelling**:

Both repos are solo-owned, and **GitHub does not notify you about your own
activity.** So the inbox slice 2 would build may well render empty on this
machine, and an empty panel reads as a broken feature. Ideas #1 and #3 need no
token, no network round-trip for #1, and #1 already has a confirmed hit.

Plan 053's maintenance note already requires this be answered before slice 2 is
built: with the token connected, one throwaway command printing the row *count*
per repo settles it in thirty seconds. **That measurement is the gate, and it
needs the maintainer to connect a token first.** Until then slice 2 is blocked
on evidence, not on effort.

## Rejected, with reasons

- **Reading local AI-assistant session history** to show "last AI session on
  this project." Technically possible, genuinely novel, and **rejected**: it
  means reading transcripts that can contain anything the user typed anywhere,
  for a cosmetic timestamp. Not a good trade, and not something to build
  without an explicit, informed ask.
- **A generic plugin system.** Nothing in the app has two implementations of
  anything yet. A plugin system built before the second implementation exists
  is an abstraction over a sample size of one.
- **Anything that posts, pushes, or deploys on the user's behalf.** Hangar
  reads and reports. The moment it writes to a remote, every bug becomes
  someone else's problem too.
