# Plan 064: Toasts that leave, and a bell that remembers

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result. If a STOP condition
> occurs, stop and report — do not improvise. Update this plan's row in
> `plans/README.md` when done.
>
> **Drift check**: `grep -n "toast: string | null" src/store.ts && grep -n "toastTone" src/store.ts && grep -n "toastProjectId\|Show logs" src/App.tsx`
> All must match. On a mismatch, STOP.
>
> **Gate ownership**: you run `cargo check --all-targets`, `cargo check
> --no-default-features --all-targets`, `cargo test` and `npm run typecheck`.
> Your reviewer runs `npm run build` and the bundle.

## Status

- **Priority**: **P1** — reported by the maintainer from real use
- **Effort**: M
- **Risk**: MED — §11 gains a header control, and toasts carry §7's only error
  surface. Losing an error is worse than the bug being fixed.
- **Depends on**: nothing
- **Category**: bug + feature
- **Planned at**: 2026-08-11

## The report, verbatim

> *"the notification in the system like, port is taken or u grouped, or u
> ungrouped projects stays forever until i close it, it should stay for 3~5
> seconds then if i want to see i can open the ring icon to see notifications
> for the system"*

He is right, and the current code confirms it: `toast: string | null` in
`src/store.ts` is set and then **never cleared except by the user**. There is no
timer anywhere in the frontend — `grep -riE "setTimeout|autoDismiss|duration"
src/App.tsx src/store.ts` returns nothing.

So *"Moved to folder"* — a confirmation of something the user just did and
already knows — sits on screen until clicked. That is the interaction cost of an
error applied to a success.

## The two halves, and why both are required

**Auto-dismiss alone would be a regression.** §7 makes toasts the *only* surface
for every command error, and §9/§12 put real, actionable text in them:

- *"Port 3000 is in use by node (PID 4321) — is this project running
  elsewhere?"* — which plan 047 gave a **Ports** button.
- *"Install failed (exit 1) — see the log, then Run again."* — which plan 034
  gave a **Show logs** button.

Dismiss those after four seconds with nowhere to retrieve them and the app has
*lost* information it used to keep. **The bell is not a nice extra; it is the
thing that makes auto-dismiss safe.** Build both or neither.

## Behaviour

### The toast

- **Auto-dismisses.** Neutral tone: **4 s**. Error tone: **6 s** — an error
  carries more words and a possible action, and needs longer to read. Both sit
  inside the "3~5 seconds" the maintainer asked for, near enough; if you deviate
  further, say why.
- **Hover pauses the timer**, and it resumes on leave. A message that vanishes
  mid-sentence while being read is the single most irritating version of this
  feature. Keyboard focus inside the toast pauses it too.
- The manual dismiss button **stays**.
- A new toast replaces the current one, as today, and resets the timer.

### The bell

- A quiet header control, next to the existing ones. **An unread count when
  there are unread entries**, nothing when there are none.
- Opens a slide-over on exactly the terms §11 already sets for Ports, Doctor
  and Inbox: reads local state only, no network, Esc closes, added to the
  `inert` overlay set and the folder band's Esc guard.
- **Newest first** — this is the one panel in the app that is *not* in
  `projects.json` array order, because it is a chronology, not a registry.
  Say so in the §11 amendment so it does not read as an oversight.
- Each entry keeps its **text, tone, relative time, and its action button** —
  the Ports and Show-logs buttons must work from the panel exactly as they do
  from the toast. That is the whole point.
- **Capped** at a fixed recent count (100 is ample). Oldest fall off.
- **Ephemeral.** In memory only — **nothing is written to disk.** Do not add a
  field to `projects.json` or `settings.json`, and do not create a new file.
  §5's data model is not changing for a notification log.

## Scope

**In scope**: `src/store.ts` (the entry list, the timer, unread count),
`src/App.tsx` (the bell, the mount, the `inert` set), a new
`src/components/NotificationsPanel.tsx`, `src/components/ProjectGrid.tsx`
(the folder Esc guard), `SPEC.md` §11, and a **zero-import leaf module** for any
logic worth testing (see below).

**Out of scope** (do NOT build):
- **Any Rust change.** `git diff --stat` must contain no `.rs` file. This is
  entirely frontend; the backend already emits everything needed.
- Persisting notifications across restarts.
- Grouping, filtering, search, severity levels beyond the existing two tones.
- Desktop/OS notifications, sound, badges on the dock. §3.
- Any change to what any toast *says*, or to which code paths raise one.
- Any new dependency.

## Testing

Put the logic in a **zero-import leaf** (`src/notifications.ts`), matching
`src/launchLine.ts` and `src/session.ts`, with a `node --test` file. Test:

1. A new entry is prepended, not appended.
2. The cap drops the oldest, never the newest.
3. The unread count is the number of entries added since the panel was last
   opened — and **opening the panel zeroes it**.
4. An entry carrying a project id keeps it, so its action button still works.
5. Two identical messages in a row are two entries, not one — **do not
   deduplicate**; "port is taken" twice means it happened twice.

Then **mutation-test #3**: make opening the panel not clear the count, confirm
the test goes red, restore. A badge that never clears is the failure mode users
report as "the notification thing is broken".

Rust test count **must not change** (226 passed / 3 ignored today).

## Done criteria

- [ ] Four gates green; `cargo test` count **unchanged**
- [ ] `git diff --stat` contains **no `.rs` file** — paste it
- [ ] A toast auto-dismisses; hovering it pauses the timer
- [ ] Every dismissed toast is retrievable from the bell, **with its action
      button still working**
- [ ] Nothing is written to disk — no new file, no new settings key
- [ ] The mutation test in #3 was run — report both outcomes
- [ ] §11 amended; `plans/README.md` row updated

## STOP conditions

- You find yourself adding auto-dismiss **without** the history panel, or
  shipping them in separate commits where the first leaves errors
  unretrievable. They are one change.
- A toast with an action button would auto-dismiss and the action becomes
  unreachable. That is the bug this plan exists to avoid.
- You need a Rust change, a new dependency, or a new persisted field.
- The bell would poll, or read anything over the network.

## Maintenance notes

- The rule to preserve: **§7 says errors surface as toasts, and that is still
  true — the bell is where a toast goes, not a replacement for it.** If a future
  change ever raises an error *only* into the bell, an error can be missed
  entirely, which is strictly worse than today's behaviour.
- Watch for the temptation to deduplicate repeated messages. Two identical
  "port is in use" toasts are two real events, and collapsing them hides a
  retry loop.
