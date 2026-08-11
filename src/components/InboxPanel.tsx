/**
 * SPEC.md §18 / §11's Inbox entry — the Inbox slide-over. Plan 053 built the connection surface;
 * **plan 062 filled it in with build state rather than a notification list**, and the reason is a
 * measurement, not a preference: on 2026-08-11 the live account had 50 unread notifications, all
 * 50 of them CI results, and 0 issues, 0 pull requests, 0 discussions. Listing them would have
 * produced fifty identical rows saying "CI workflow run failed". One line per repository — is the
 * build red or green — is the fact anyone acts on.
 *
 * Two things this panel renders, in this order:
 *
 * 1. the connection state — disconnected, keychain-denied, connected, and every network-derived
 *    state `GithubStatus` can carry (invalid, insufficient-scope, rate-limited,
 *    secondary-rate-limited, offline). **None of them is a toast, none is an error** (§11): they
 *    render in place, from the `GithubStatus` the backend returned;
 * 2. one row per distinct repository. §11: the unit is the repository, never the project, and a
 *    project that is not on GitHub, has no remote, or cannot be seen with the current token is
 *    simply **absent** — no row, no error, no toast.
 *
 * **`unknown` is never drawn as green.** It gets the muted colour and the word, exactly the rule
 * §11 states for the launch line and the Doctor panel: a check that could not run must never
 * render as a clean bill of health.
 *
 * Read-only, like Doctor: the controls here are Refresh, Connect, Disconnect, Close and Esc.
 * Nothing writes to GitHub, and nothing marks anything read.
 *
 * Modelled on `LogPanel.tsx`'s shell — same backdrop, Esc, click-away close.
 */
import { useEffect, useState } from "react";
import {
  closeInbox,
  connectGithubAction,
  disconnectGithubAction,
  githubCredentialUsable,
  refreshInbox,
  useHangarStore,
} from "../store";
import type { BuildState, GithubStatus, ProjectView, RepoBuild } from "../types";

/** §18: "state plainly in the UI" that `repo` is broad and `public_repo` is the narrower option. */
const SCOPES_NOTE =
  "Hangar asks for two scopes: notifications, and repo — full read/write access to your " +
  "repositories' code. If every repository you want to see is public, public_repo (read/write " +
  "to public repos only) is enough instead.";

/**
 * §11's palette, functional only — every value is an existing token and no new one is introduced.
 * `running` reuses `--color-status-active`, which §11 already assigns to *transitional* statuses; a
 * build in flight is the same kind of fact, and it is the only entry here that touches the accent
 * hue at all.
 *
 * `unknown` and `no-checks` share the muted colour that every other "nothing to say" line in this
 * app uses, **and** they carry their own words. That pairing is the point: colour alone would leave
 * "could not check" one glance away from green for anyone who reads shape faster than hue.
 */
const BUILD_TONE: Record<BuildState, string> = {
  passing: "text-status-running",
  failing: "text-status-danger",
  running: "text-status-active",
  "no-checks": "text-muted",
  unknown: "text-muted",
};

const BUILD_LABEL: Record<BuildState, string> = {
  passing: "passing",
  failing: "failing",
  running: "running",
  "no-checks": "no checks",
  unknown: "unknown",
};

/** One repository. The repository is the unit (§11), so the project names are the secondary line —
 *  two cards sharing a repo root are named on one row, not given a row each. */
function BuildRow({ build, projects }: { build: RepoBuild; projects: ProjectView[] }) {
  const names = build.projectIds
    .map((id) => projects.find((p) => p.id === id)?.name)
    .filter((name): name is string => Boolean(name));
  const extra =
    build.state === "unknown" && build.resetAt
      ? ` Resets ${new Date(build.resetAt).toLocaleTimeString([], { hour12: false })}.`
      : build.state === "unknown" && build.retryAfterSec !== undefined
        ? ` Try again in ${build.retryAfterSec}s.`
        : "";
  return (
    <li className="border-b border-white/5 px-5 py-3 last:border-b-0">
      <div className="flex items-baseline justify-between gap-3">
        <p className="min-w-0 truncate text-sm text-text" title={build.repository}>
          {build.repository}
        </p>
        <span className={`shrink-0 text-xs font-medium ${BUILD_TONE[build.state]}`}>
          {BUILD_LABEL[build.state]}
        </span>
      </div>
      <p className="mt-1 truncate font-mono text-xs text-muted">
        {build.branch ?? "branch unknown"}
        {names.length > 0 && ` · ${names.join(" · ")}`}
      </p>
      {build.detail && (
        <p className="mt-1 text-xs text-muted">
          {build.detail}
          {extra}
        </p>
      )}
    </li>
  );
}

function ConnectedView({
  status,
  onDisconnect,
  disconnecting,
}: {
  status: GithubStatus;
  onDisconnect: () => void;
  disconnecting: boolean;
}) {
  return (
    <div className="flex items-baseline justify-between gap-3 px-5 py-3">
      <p className="min-w-0 truncate text-xs text-muted">
        Connected as <span className="text-text">@{status.username}</span>
        {status.scopes && status.scopes.length > 0 && (
          <span className="font-mono"> · {status.scopes.join(", ")}</span>
        )}
      </p>
      <button
        type="button"
        onClick={onDisconnect}
        disabled={disconnecting}
        className="shrink-0 rounded-md border border-white/10 px-3 py-1 text-xs text-muted transition-colors hover:bg-white/5 hover:text-text disabled:cursor-not-allowed disabled:opacity-60"
      >
        {disconnecting ? "Disconnecting…" : "Disconnect"}
      </button>
    </div>
  );
}

/** SPEC.md §18's ratified failure states, each carrying its own `detail` from the backend
 *  (`commands::status_from_error` / `GithubStatus::keychain_denied`) — this just renders it with
 *  the right tone. `rate-limited`/`secondary-rate-limited` append the reset time / wait, which
 *  the backend supplies rather than this component computing (§18: "rate limits are shown, never
 *  swallowed... says when it resets"). */
function StatusBanner({ status }: { status: GithubStatus }) {
  if (status.state === "disconnected" || status.state === "connected" || !status.detail) return null;
  const extra =
    status.state === "rate-limited" && status.resetAt
      ? ` Resets ${new Date(status.resetAt).toLocaleTimeString([], { hour12: false })}.`
      : status.state === "secondary-rate-limited" && status.retryAfterSec !== undefined
        ? ` Try again in ${status.retryAfterSec}s.`
        : "";
  const isDenial = status.state === "keychain-denied";
  return (
    <p
      className={`rounded-md border px-3 py-2 text-sm ${
        isDenial
          ? "border-status-danger/40 bg-status-danger/10 text-status-danger"
          : "border-white/10 bg-white/[0.02] text-muted"
      }`}
    >
      {status.detail}
      {extra}
    </p>
  );
}

/** The Connect/Reconnect form — the only route into the feature. `hadStoredToken` on an
 *  `invalid` status is what tells this apart from a first-time Connect (§18 / plan 053's ratified
 *  "expired or revoked" wording): a token that used to work gets "Reconnect", not "Connect". */
function ConnectForm({ status }: { status: GithubStatus | null }) {
  const [token, setToken] = useState("");
  const [connecting, setConnecting] = useState(false);
  const isReconnect = status?.state === "invalid" && status.hadStoredToken === true;

  async function handleSubmit(): Promise<void> {
    if (!token.trim() || connecting) return;
    setConnecting(true);
    const ok = await connectGithubAction(token);
    setConnecting(false);
    if (ok) setToken("");
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        void handleSubmit();
      }}
      className="space-y-3"
    >
      <label className="block text-sm text-muted" htmlFor="github-token">
        {isReconnect ? "Paste a new personal access token" : "Personal access token"}
      </label>
      <input
        id="github-token"
        type="password"
        autoComplete="off"
        spellCheck={false}
        value={token}
        onChange={(e) => setToken(e.target.value)}
        placeholder="ghp_…"
        className="w-full rounded-md border border-white/10 bg-bg px-3 py-2 font-mono text-sm text-text outline-none focus:border-accent"
      />
      <p className="text-xs text-muted">{SCOPES_NOTE}</p>
      <button
        type="submit"
        disabled={!token.trim() || connecting}
        className="rounded-md bg-accent px-4 py-2 text-sm font-semibold text-bg transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {connecting ? "Connecting…" : isReconnect ? "Reconnect" : "Connect"}
      </button>
    </form>
  );
}

export function InboxPanel() {
  const { inboxOpen, githubStatus, builds, buildsPending, projects } = useHangarStore();
  const [disconnecting, setDisconnecting] = useState(false);

  // §11: Esc closes the slide-over — the only keyboard shortcut in v0, same as every other panel.
  useEffect(() => {
    if (!inboxOpen) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") closeInbox();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [inboxOpen]);

  useEffect(() => {
    if (!inboxOpen) setDisconnecting(false);
  }, [inboxOpen]);

  async function handleDisconnect(): Promise<void> {
    setDisconnecting(true);
    await disconnectGithubAction();
    setDisconnecting(false);
  }

  if (!inboxOpen) return null;

  const connected = githubStatus?.state === "connected";
  // The same predicate the store fetches on — imported, never re-derived here, so this panel can
  // never render rows for a credential state the store decided not to fetch for.
  const credentialUsable = githubCredentialUsable(githubStatus);
  // §11: a stale green is worse than no green, so the header says when the snapshot was taken —
  // and while a read is in flight the honest header is that it is being taken, not a stale time.
  const asOf = builds ? new Date(builds.checkedAt).toLocaleTimeString([], { hour12: false }) : "—";

  return (
    <div className="fixed inset-0 z-20 flex justify-end" role="presentation">
      <div className="hangar-fade-in flex-1 bg-black/40" onClick={closeInbox} aria-hidden="true" />
      <aside
        role="dialog"
        aria-modal="true"
        aria-label="Inbox"
        className="hangar-slide-in flex h-full w-[min(28rem,92vw)] flex-col border-l border-white/10 bg-surface shadow-2xl"
      >
        <header className="flex items-center justify-between gap-3 border-b border-white/5 px-5 py-4">
          <div className="min-w-0">
            <h2 className="font-display text-base font-medium text-text">Inbox</h2>
            {credentialUsable && (
              <p className="mt-0.5 font-mono text-xs text-muted">
                {buildsPending ? "checking…" : `as of ${asOf}`}
              </p>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {/* §11's snapshot rule: read on open and on Refresh, never polled. Hidden until there
                is a credential to refresh with — the Connect button is the action before that. */}
            {credentialUsable && (
              <button
                type="button"
                disabled={buildsPending}
                onClick={() => void refreshInbox()}
                className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-muted"
              >
                Refresh
              </button>
            )}
            <button
              type="button"
              aria-label="Close inbox"
              onClick={closeInbox}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              <span aria-hidden="true">✕</span>
            </button>
          </div>
        </header>

        {!githubStatus ? (
          <p className="px-5 py-4 text-sm text-muted">Checking connection…</p>
        ) : !credentialUsable ? (
          <div className="space-y-4 px-5 py-4">
            <StatusBanner status={githubStatus} />
            <ConnectForm status={githubStatus} />
          </div>
        ) : (
          <>
            <div className="border-b border-white/5">
              {connected ? (
                <ConnectedView
                  status={githubStatus}
                  onDisconnect={() => void handleDisconnect()}
                  disconnecting={disconnecting}
                />
              ) : (
                // §18: offline and rate-limited render in place, above the rows they explain —
                // never as a toast, and never in place of the rows.
                <div className="px-5 py-3">
                  <StatusBanner status={githubStatus} />
                </div>
              )}
            </div>
            {/* One row per distinct repository, in the array order the backend returned — never
                sorted by state, for the same reason the grid and the Ports panel are never
                re-sorted. "Not fetched yet" and "fetched, and there are none" are different facts
                and say different things; conflating them would state "No repositories" for as long
                as the network call takes, to a user who does have them. */}
            <ul className="flex-1 overflow-y-auto">
              {builds === null ? (
                <li className="px-5 py-4 text-sm text-muted">
                  {buildsPending ? "Checking builds…" : "Nothing checked yet."}
                </li>
              ) : builds.repos.length === 0 ? (
                // Deliberately states the conditions rather than asserting a reason: a repository
                // is absent when its `origin` is not GitHub, when this token cannot see it, AND
                // when the checked-out branch does not exist on GitHub (an unpushed branch answers
                // exactly like an invisible repository). Naming one of those three as *the* cause
                // would be a guess printed as a fact.
                <li className="px-5 py-4 text-sm text-muted">
                  Nothing to show. A project appears here once its <code>origin</code> is a GitHub
                  repository this token can see, on a branch that exists there.
                </li>
              ) : (
                builds.repos.map((build) => (
                  <BuildRow
                    key={`${build.repository}@${build.branch ?? "?"}`}
                    build={build}
                    projects={projects}
                  />
                ))
              )}
            </ul>
            {/* Saying what this panel is, where the user can see it: it reports one fact per
                repository and changes nothing on GitHub — §18's line, made visible. */}
            <footer className="border-t border-white/5 px-5 py-3 text-xs text-muted">
              Hangar only reads here — it posts nothing and marks nothing as read. Conversations are
              deferred until there are any.
            </footer>
          </>
        )}
      </aside>
    </div>
  );
}

export default InboxPanel;
