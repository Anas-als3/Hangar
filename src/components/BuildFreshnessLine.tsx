/**
 * SPEC.md §11 "Build freshness" (added 2026-08-11, plan 063) — one line, above the launch line,
 * shown only when the installed bundle is newer than the process rendering it.
 *
 * Modelled on `ResumeLine` in `App.tsx` and `LaunchLine` beside it: the same quiet bordered strip,
 * the same `null` return when there is nothing to say. Silent otherwise — zero pixels, no empty
 * state, no "you are up to date" badge — and on any machine where the app was not installed from a
 * local build, silent forever.
 *
 * # Why it exists
 *
 * Hangar is developed on the machine it runs on and installed to `/Applications`. Replacing the
 * bundle does nothing to a process already running, so three times a merged feature has looked
 * *missing* when it had shipped — most recently a build installed at 09:11, reported missing from a
 * window that had been open since 06:54. That is the most expensive wrong conclusion available: it
 * sends someone debugging code that is correct.
 *
 * # What this component may do
 *
 * **Text. That is the whole element.** No "Restart now", no kill, no relaunch, no update download,
 * no version check against a server. SPEC.md §3 bans auto-update outright, and §8's guarantee is
 * that Hangar owns its children's lifecycle — a restart button that silently killed a running dev
 * server would be a §6/§8 violation wearing a convenience hat. If a button ever appears in this
 * file, that is the thing to reject in review. The user restarts Hangar.
 *
 * **It must never claim to be stale when it is not.** A false nag teaches the user to ignore the
 * line, and then it is worse than absent — the next real one is ignored too. Every silence rule
 * lives in `freshness.rs` (a tolerance, not a strict `>`; two facts captured before anything can
 * move them; `Ok`, never `Err`), and this file's only job is not to invent a reason to render:
 * `newerBuildInstalled` is the single condition, and `null` before the first read is not it.
 */
import { relativeTime } from "../status";
import { useHangarStore } from "../store";

export function BuildFreshnessLine() {
  const { buildFreshness } = useHangarStore();
  // `null` is "not read yet", which is not evidence of anything. Same shape as every other
  // silent-when-clean element: one condition, and no second way to become visible.
  if (!buildFreshness?.newerBuildInstalled) return null;

  return (
    <div className="mb-4 flex items-center gap-3 rounded-md border border-white/10 bg-white/[0.02] px-4 py-2.5 text-sm">
      <span className="shrink-0 text-muted">Hangar</span>
      <p className="min-w-0 truncate text-text">
        A newer build is installed. Restart Hangar to use it.
      </p>
      {buildFreshness.installedAt && (
        <span
          className="ml-auto shrink-0 text-xs text-muted/70"
          title={buildFreshness.installedAt}
        >
          installed {relativeTime(buildFreshness.installedAt)}
        </span>
      )}
    </div>
  );
}

export default BuildFreshnessLine;
