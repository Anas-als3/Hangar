/**
 * SPEC.md §11 "Launch line" (added 2026-08-11, plan 060) — one line under the header naming what
 * needs the user, rendered only when there is something to name.
 *
 * Modelled on `ResumeLine` in `App.tsx`: the same quiet bordered strip, the same "at most three,
 * then `+N`" cap, the same `null` return when there is nothing to say. It sits as a sibling to that
 * line rather than on the cards, because §11's card element list is fixed and short and a git line
 * on every card would put a permanent element on cards with nothing to report.
 *
 * # What this component may do
 *
 * **It reports; it never acts (§3).** There is no push, pull, fetch, commit or stash control here,
 * and there must never be one — not behind a confirm, not in a menu. The single action is
 * `revealProject`, which scrolls a card into view. If a "Push" button ever appears in this file,
 * Hangar has become a git client with rounded corners, which §3 says it is not.
 *
 * **"Behind" is not rendered, because it is not known.** Hangar does not fetch. `launchLine.ts` has
 * no input capable of carrying a behind count, and this file has nothing to print it with.
 *
 * **A check that could not run is stated, not swallowed.** `notChecked` renders as a trailing
 * `N not checked` fragment rather than as items, so a broken git can never push a real finding past
 * the three-item cap — and can never be mistaken for a clean library either.
 */
import { hasSomethingToSay, launchLine, MAX_ITEMS } from "../launchLine";
import { relativeTime } from "../status";
import { revealProject, useHangarStore } from "../store";
import type { ProjectView } from "../types";

export function LaunchLine({ projects }: { projects: ProjectView[] }) {
  const { vcs } = useHangarStore();
  const result = launchLine(projects, vcs);
  if (!hasSomethingToSay(result)) return null;

  const shown = result.items.slice(0, MAX_ITEMS);
  const overflow = result.items.length - shown.length;
  // One shared timestamp for the whole snapshot (§7's `checkedAt`), so the line can say how old it
  // is instead of implying it is live. It is a snapshot, not a monitor.
  const checkedAt = vcs && vcs.length > 0 ? vcs[0].checkedAt : null;
  // Named in the hover text so "1 not checked" is never a dead end.
  const notCheckedTitle =
    result.notChecked.length > 0
      ? `Hangar could not check: ${result.notChecked.join(", ")}`
      : undefined;

  return (
    <div className="mb-4 flex items-center gap-3 rounded-md border border-white/10 bg-white/[0.02] px-4 py-2.5 text-sm">
      <span className="shrink-0 text-muted">Needs you</span>
      <p className="min-w-0 truncate text-muted">
        {shown.map((item, i) => (
          <span key={item.projectId}>
            {i > 0 && <span aria-hidden="true"> · </span>}
            <button
              type="button"
              title={item.title}
              onClick={() => revealProject(item.projectId)}
              className="rounded text-text underline-offset-2 transition-colors hover:underline"
            >
              {item.name} <span className="text-muted">· {item.detail}</span>
            </button>
          </span>
        ))}
        {overflow > 0 && <span> · +{overflow}</span>}
        {result.notChecked.length > 0 && (
          <span title={notCheckedTitle}>
            {shown.length > 0 || overflow > 0 ? " · " : ""}
            {result.notChecked.length} not checked
          </span>
        )}
      </p>
      {checkedAt && (
        <span className="ml-auto shrink-0 text-xs text-muted/70" title={checkedAt}>
          {relativeTime(checkedAt)}
        </span>
      )}
    </div>
  );
}

export default LaunchLine;
