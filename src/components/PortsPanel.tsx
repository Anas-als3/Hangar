/**
 * SPEC.md §11 — the Ports slide-over (added 2026-08-10, plan 041): one row per registered
 * project, in the same array order `get_port_status` returns, showing what is holding its
 * pinned port. Modelled on `LogPanel.tsx`'s shell — read that first.
 *
 * Read-only except for one button: Stop, which calls the SAME `stopProjectAction` the card's own
 * Stop button calls, and only ever with a project id — it must never read a PID off this panel.
 * That is the structural safety property that keeps a diagnostic panel from becoming an acting
 * one. Plan 042's "Free the port" action is a deliberately separate addition with its own gates;
 * this panel only offers **Copy `kill <pid>`** for a foreign holder.
 */
import { useEffect, useState } from "react";
import { STATUS_LABEL, STATUS_TONE } from "../status";
import { closePorts, freePortAction, refreshPorts, stopProjectAction, useHangarStore } from "../store";
import type { PortHolder, PortStatus, ProjectView } from "../types";

/** Same clipboard-then-`execCommand`-fallback idiom as `LogPanel.tsx`'s Copy button. */
async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    textarea.select();
    let ok = false;
    try {
      ok = document.execCommand("copy");
    } catch {
      ok = false;
    }
    document.body.removeChild(textarea);
    return ok;
  }
}

type RowState = "free" | "managed" | "not-managed" | "unknown";

/** §11's four row states. `unknown` (listenerCount === 0) is checked before `not-managed` so an
 *  unidentifiable owner is never mislabelled as a specific "not managed by Hangar" claim. */
function rowState(port: PortStatus, project: ProjectView | undefined): RowState {
  if (!port.busy) return "free";
  if (project && project.status !== "stopped" && project.status !== "crashed") return "managed";
  if (port.listenerCount === 0) return "unknown";
  return "not-managed";
}

function rowLabel(state: RowState, project: ProjectView | undefined): string {
  if (state === "managed" && project) return STATUS_LABEL[project.status];
  if (state === "unknown") return "In use — owner unknown";
  if (state === "not-managed") return "In use — not managed by Hangar";
  return "Free";
}

function rowTone(state: RowState, project: ProjectView | undefined): string {
  if (state === "managed" && project) return STATUS_TONE[project.status];
  if (state === "not-managed") return "text-text";
  return "text-muted";
}

/** Plan 042: the frontend's mirror of `free_port_gate`'s gates 1-4 — offer the button ONLY when
 *  Rust would not immediately refuse it. This is advisory (Rust re-verifies everything, including
 *  gate 5, inside `free_port` itself) — it exists purely so the button never appears somewhere it
 *  is certain to be rejected. Gate 3 (not a Hangar-managed project) is `rowState`'s job already —
 *  `not-managed` is the only state this is ever consulted from. */
function canFreePort(state: RowState, holder: PortHolder | undefined): boolean {
  if (state !== "not-managed" || !holder) return false;
  return holder.sameUser === true && !!holder.command && !!holder.startedAt;
}

/** §11: "renders whenever the port is busy and exactly one listener parsed — in every state,
 *  including Hangar's own." The listenerCount 0/>1 branches are the same rule's other cases —
 *  explanatory text instead of a specific process. `offerCopy` further restricts the Copy button
 *  to the "not managed by Hangar" row; the other three states never show it even when a holder
 *  parsed, because the panel's only acting control besides Stop is this one. */
function HolderDetail({
  port,
  offerCopy,
  offerFree,
  copiedPid,
  onCopyKill,
  onFreeClick,
}: {
  port: PortStatus;
  offerCopy: boolean;
  offerFree: boolean;
  copiedPid: number | null;
  onCopyKill: (pid: number) => void;
  onFreeClick: (holder: PortHolder) => void;
}) {
  if (!port.busy) return null;
  if (port.listenerCount > 1) {
    return (
      <p className="mt-1.5 text-xs text-muted">
        {port.listenerCount} processes are listening on this port — Hangar will not guess which
        one.
      </p>
    );
  }
  if (!port.holder) {
    return (
      <p className="mt-1.5 text-xs text-muted">
        Hangar could not identify the owner. Processes owned by another user or by the system are
        not visible to it.
      </p>
    );
  }
  const holder = port.holder;
  return (
    <div className="mt-1.5 space-y-1 text-xs">
      <p className="font-mono text-muted">
        {holder.name} · PID {holder.pid}
        {holder.startedAt ? ` · started ${holder.startedAt}` : ""}
      </p>
      {holder.command && (
        <p className="truncate font-mono text-muted" title={holder.command}>
          {holder.command}
        </p>
      )}
      {holder.parentExited && (
        <p className="text-status-danger/80">its parent has exited — nothing is supervising it</p>
      )}
      {/* Plan 042: Free sits AFTER Copy, in the row's trailing corner, and is deliberately styled
          quieter — no border — since it is the one thing on this panel that is hard to undo. */}
      {(offerCopy || offerFree) && (
        <div className="flex items-center gap-2">
          {offerCopy && (
            <button
              type="button"
              onClick={() => onCopyKill(holder.pid)}
              className="mt-0.5 rounded-md border border-white/10 px-2.5 py-1 text-xs text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              {copiedPid === holder.pid ? "Copied" : `Copy kill ${holder.pid}`}
            </button>
          )}
          {offerFree && (
            <button
              type="button"
              onClick={() => onFreeClick(holder)}
              className="mt-0.5 rounded-md px-2.5 py-1 text-xs text-muted/70 transition-colors hover:bg-white/5 hover:text-muted"
            >
              Free the port
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/** Plan 042: what the confirm needs to render — the port and the holder it names. Kept as one
 *  object rather than two pieces of state so the dialog can never open with one set and not the
 *  other (see `handleFreeClick`'s gate-4 check below). */
interface FreeTarget {
  port: PortStatus;
  holder: PortHolder;
}

export function PortsPanel() {
  const { portsOpen, ports, projects } = useHangarStore();
  const [copiedPid, setCopiedPid] = useState<number | null>(null);
  const [freeTarget, setFreeTarget] = useState<FreeTarget | null>(null);
  const [freeing, setFreeing] = useState(false);

  // §11: Esc closes the slide-over — the only keyboard shortcut in v0. Plan 042: while the confirm
  // is open, Esc dismisses IT first (same layered-dialog behaviour as every other confirm here),
  // never both at once.
  useEffect(() => {
    if (!portsOpen) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      if (freeTarget) {
        if (!freeing) setFreeTarget(null);
        return;
      }
      closePorts();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [portsOpen, freeTarget, freeing]);

  // A freshly opened panel never shows a stale "Copied" from the last time it was open, and never
  // leaves a confirm from a previous open hanging around.
  useEffect(() => {
    setCopiedPid(null);
    setFreeTarget(null);
    setFreeing(false);
  }, [portsOpen]);

  async function handleCopyKill(pid: number): Promise<void> {
    const ok = await copyText(`kill ${pid}`);
    if (!ok) return;
    setCopiedPid(pid);
    setTimeout(() => setCopiedPid(null), 1500);
  }

  // Gate 4, defensively: the confirm must never render without a full command line, so this
  // refuses to open it even if a caller ever wired the button up wrong.
  function handleFreeClick(port: PortStatus, holder: PortHolder): void {
    if (!holder.command) return;
    setFreeTarget({ port, holder });
  }

  // §9 step 1: no auto-Run afterwards — this only calls `freePortAction`, never `startProject`.
  async function handleConfirmFree(): Promise<void> {
    if (!freeTarget) return;
    setFreeing(true);
    await freePortAction(freeTarget.port.projectId, freeTarget.holder.pid, freeTarget.port.port);
    setFreeing(false);
    setFreeTarget(null);
  }

  if (!portsOpen) return null;

  // Every row shares one `checkedAt` (one snapshot per `get_port_status` call) — §11: "the header
  // states when the snapshot was taken".
  const checkedAt = ports?.[0]?.checkedAt;
  const asOf = checkedAt ? new Date(checkedAt).toLocaleTimeString([], { hour12: false }) : "—";

  return (
    <>
    <div className="fixed inset-0 z-20 flex justify-end" role="presentation">
      {/* Click-away closes, same as Esc. §11 enter transition: backdrop fades in. */}
      <div className="hangar-fade-in flex-1 bg-black/40" onClick={closePorts} aria-hidden="true" />

      <aside
        role="dialog"
        aria-modal="true"
        aria-label="Ports"
        className="hangar-slide-in flex h-full w-[min(34rem,92vw)] flex-col border-l border-white/10 bg-surface shadow-2xl"
      >
        <header className="flex items-center justify-between gap-3 border-b border-white/5 px-5 py-4">
          <div className="min-w-0">
            <h2 className="font-display text-base font-medium text-text">Ports</h2>
            <p className="mt-0.5 font-mono text-xs text-muted">as of {asOf}</p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              onClick={() => void refreshPorts()}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              Refresh
            </button>
            <button
              type="button"
              aria-label="Close ports"
              onClick={closePorts}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              <span aria-hidden="true">✕</span>
            </button>
          </div>
        </header>

        {/* One row per project, in the array order `get_port_status` returned — never sorted by
            port or state (§11, same reasoning as the grid never re-sorting). */}
        <ul className="flex-1 overflow-y-auto">
          {!ports || ports.length === 0 ? (
            <li className="px-5 py-4 text-sm text-muted">No registered projects.</li>
          ) : (
            ports.map((port) => {
              const project = projects.find((p) => p.id === port.projectId);
              const state = rowState(port, project);
              return (
                <li key={port.projectId} className="border-b border-white/5 px-5 py-3 last:border-b-0">
                  <div className="flex items-center justify-between gap-3">
                    <div className="flex min-w-0 items-center gap-3">
                      <span className="font-mono text-xs text-muted">:{port.port}</span>
                      <span className="truncate text-sm text-text">
                        {project?.name ?? port.projectId}
                      </span>
                    </div>
                    <div className="flex shrink-0 items-center gap-2">
                      <span className={`text-xs font-medium ${rowTone(state, project)}`}>
                        {rowLabel(state, project)}
                      </span>
                      {state === "managed" && project && (
                        <button
                          type="button"
                          disabled={project.status === "stopping"}
                          onClick={() => void stopProjectAction(project.id)}
                          className="rounded-md border border-white/10 px-3 py-1.5 text-xs text-text transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-60"
                        >
                          {project.status === "stopping" ? "Stopping…" : "Stop"}
                        </button>
                      )}
                    </div>
                  </div>
                  <HolderDetail
                    port={port}
                    offerCopy={state === "not-managed"}
                    offerFree={canFreePort(state, port.holder)}
                    copiedPid={copiedPid}
                    onCopyKill={(pid) => void handleCopyKill(pid)}
                    onFreeClick={(holder) => handleFreeClick(port, holder)}
                  />
                </li>
              );
            })
          )}
        </ul>
      </aside>
    </div>

    {/* Plan 042: the confirm shell — modelled on `AddEditDialog.tsx`'s idiom (backdrop,
        `hangar-dialog-in`, its own Esc listener below), never `window.confirm`. Cannot render
        without `freeTarget.holder.command` (gate 4) — `handleFreeClick` refuses to set it
        otherwise. */}
    {freeTarget && (
      <div className="fixed inset-0 z-30 flex items-center justify-center" role="presentation">
        <div
          className="hangar-fade-in absolute inset-0 bg-black/40"
          onClick={() => !freeing && setFreeTarget(null)}
          aria-hidden="true"
        />
        <div
          role="dialog"
          aria-modal="true"
          aria-label={`Free port ${freeTarget.port.port}`}
          className="hangar-dialog-in relative z-10 w-[min(28rem,92vw)] rounded-lg border border-white/10 bg-surface p-6 shadow-2xl"
        >
          <h2 className="font-display text-lg font-medium text-text">
            Free port {freeTarget.port.port}?
          </h2>
          <p className="mt-3 text-sm text-muted">
            This sends SIGTERM to one process. Hangar did not start it.
          </p>
          <div className="mt-4 space-y-1 rounded-md border border-white/10 bg-bg px-3 py-2.5 font-mono text-xs text-muted">
            <p>
              {freeTarget.holder.name} · PID {freeTarget.holder.pid}
            </p>
            {freeTarget.holder.startedAt && <p>started {freeTarget.holder.startedAt}</p>}
            <p className="break-all">{freeTarget.holder.command}</p>
            {freeTarget.holder.parentExited && (
              <p className="text-status-danger/80">
                its parent has exited — nothing is supervising it
              </p>
            )}
          </div>
          <div className="mt-6 flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setFreeTarget(null)}
              disabled={freeing}
              className="rounded-md border border-white/10 px-4 py-2 text-sm text-text transition-colors hover:bg-white/5 disabled:cursor-not-allowed disabled:opacity-60"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => void handleConfirmFree()}
              disabled={freeing}
              className="rounded-md border border-status-danger/50 px-4 py-2 text-sm font-medium text-status-danger transition-colors hover:bg-status-danger/10 disabled:cursor-not-allowed disabled:opacity-60"
            >
              {freeing ? "Freeing…" : "Free the port"}
            </button>
          </div>
        </div>
      </div>
    )}
    </>
  );
}

export default PortsPanel;
