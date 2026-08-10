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
import { closePorts, refreshPorts, stopProjectAction, useHangarStore } from "../store";
import type { PortStatus, ProjectView } from "../types";

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

/** §11: "renders whenever the port is busy and exactly one listener parsed — in every state,
 *  including Hangar's own." The listenerCount 0/>1 branches are the same rule's other cases —
 *  explanatory text instead of a specific process. `offerCopy` further restricts the Copy button
 *  to the "not managed by Hangar" row; the other three states never show it even when a holder
 *  parsed, because the panel's only acting control besides Stop is this one. */
function HolderDetail({
  port,
  offerCopy,
  copiedPid,
  onCopyKill,
}: {
  port: PortStatus;
  offerCopy: boolean;
  copiedPid: number | null;
  onCopyKill: (pid: number) => void;
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
      {offerCopy && (
        <button
          type="button"
          onClick={() => onCopyKill(holder.pid)}
          className="mt-0.5 rounded-md border border-white/10 px-2.5 py-1 text-xs text-muted transition-colors hover:bg-white/5 hover:text-text"
        >
          {copiedPid === holder.pid ? "Copied" : `Copy kill ${holder.pid}`}
        </button>
      )}
    </div>
  );
}

export function PortsPanel() {
  const { portsOpen, ports, projects } = useHangarStore();
  const [copiedPid, setCopiedPid] = useState<number | null>(null);

  // §11: Esc closes the slide-over — the only keyboard shortcut in v0.
  useEffect(() => {
    if (!portsOpen) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") closePorts();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [portsOpen]);

  // A freshly opened panel never shows a stale "Copied" from the last time it was open.
  useEffect(() => {
    setCopiedPid(null);
  }, [portsOpen]);

  async function handleCopyKill(pid: number): Promise<void> {
    const ok = await copyText(`kill ${pid}`);
    if (!ok) return;
    setCopiedPid(pid);
    setTimeout(() => setCopiedPid(null), 1500);
  }

  if (!portsOpen) return null;

  // Every row shares one `checkedAt` (one snapshot per `get_port_status` call) — §11: "the header
  // states when the snapshot was taken".
  const checkedAt = ports?.[0]?.checkedAt;
  const asOf = checkedAt ? new Date(checkedAt).toLocaleTimeString([], { hour12: false }) : "—";

  return (
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
                    copiedPid={copiedPid}
                    onCopyKill={(pid) => void handleCopyKill(pid)}
                  />
                </li>
              );
            })
          )}
        </ul>
      </aside>
    </div>
  );
}

export default PortsPanel;
