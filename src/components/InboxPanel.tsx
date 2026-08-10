/**
 * SPEC.md §18 / plan 053 — the Inbox slide-over. SHELL ONLY for slice 1: §11 describes two panes
 * (a notification list, a single thread with a reply box) that slices 2/3 build; this renders
 * only the connection surface underneath them — disconnected, keychain-denied, connected, and
 * every network-derived state `GithubStatus` can carry (invalid, insufficient-scope,
 * rate-limited, secondary-rate-limited, offline). None of them is a toast, none is an error
 * (§11) — they render in place, from the `GithubStatus` the backend returned.
 *
 * Modelled on `LogPanel.tsx`'s shell — same backdrop, Esc, click-away close.
 */
import { useEffect, useState } from "react";
import { closeInbox, connectGithubAction, disconnectGithubAction, useHangarStore } from "../store";
import type { GithubStatus } from "../types";

/** §18: "state plainly in the UI" that `repo` is broad and `public_repo` is the narrower option. */
const SCOPES_NOTE =
  "Hangar asks for two scopes: notifications, and repo — full read/write access to your " +
  "repositories' code. If every repository you want to see is public, public_repo (read/write " +
  "to public repos only) is enough instead.";

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
    <div className="space-y-3">
      <p className="text-sm text-text">
        Connected as <span className="font-medium">@{status.username}</span>
      </p>
      {status.scopes && status.scopes.length > 0 && (
        <p className="font-mono text-xs text-muted">scopes: {status.scopes.join(", ")}</p>
      )}
      <button
        type="button"
        onClick={onDisconnect}
        disabled={disconnecting}
        className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-text transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-60"
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
  const { inboxOpen, githubStatus } = useHangarStore();
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
          <h2 className="font-display text-base font-medium text-text">Inbox</h2>
          <button
            type="button"
            aria-label="Close inbox"
            onClick={closeInbox}
            className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
          >
            <span aria-hidden="true">✕</span>
          </button>
        </header>

        <div className="flex-1 overflow-y-auto px-5 py-4">
          {!githubStatus ? (
            <p className="text-sm text-muted">Checking connection…</p>
          ) : githubStatus.state === "connected" ? (
            <ConnectedView status={githubStatus} onDisconnect={() => void handleDisconnect()} disconnecting={disconnecting} />
          ) : (
            <div className="space-y-4">
              <StatusBanner status={githubStatus} />
              <ConnectForm status={githubStatus} />
            </div>
          )}
        </div>
      </aside>
    </div>
  );
}

export default InboxPanel;
