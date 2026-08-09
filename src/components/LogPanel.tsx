/**
 * SPEC.md §11 — the log slide-over: mono font, autoscroll with pause-on-scroll-up, Clear,
 * stderr tinted, `system` lines muted, Esc closes.
 *
 * The panel is a *view* of the store. It registers no Tauri listener of its own — the two §7
 * listeners live in `src/store.ts` and run from app startup, so lines emitted while this panel
 * was closed are already in the store when it opens. The backfill (`get_log_buffer`) is merged
 * on open by the store.
 */
import { useEffect, useRef, useState } from "react";
import type { LogLine } from "../types";
import { clearLogs, closeLogs, useHangarStore } from "../store";

/**
 * §11 Copy button: the entire retained buffer, with stream prefixes, via the async clipboard
 * API with an `execCommand('copy')` fallback — the async API is unreliable on Linux
 * webkit2gtk builds. Resolves to whether the copy actually succeeded.
 */
async function copyLogLines(lines: LogLine[]): Promise<boolean> {
  const text = lines.map((line) => `[${line.stream}] ${line.line}`).join("\n");
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

/** §11: stderr tinted, `system` lines muted, stdout plain. Tone comes from the stream field only. */
const STREAM_TONE: Record<LogLine["stream"], string> = {
  stdout: "text-text",
  stderr: "text-status-danger",
  system: "text-muted",
};

/** Treat "within a line or two of the bottom" as being at the bottom. */
const BOTTOM_SLACK_PX = 24;

export function LogPanel() {
  const { openLogsFor, projects, logs } = useHangarStore();
  const [autoscroll, setAutoscroll] = useState(true);
  const [copied, setCopied] = useState(false);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  const project = projects.find((p) => p.id === openLogsFor) ?? null;
  const lines = openLogsFor ? (logs[openLogsFor] ?? []) : [];

  async function handleCopy(): Promise<void> {
    const ok = await copyLogLines(lines);
    if (!ok) return;
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  // §11: Esc closes the slide-over. The only keyboard shortcut in v0.
  useEffect(() => {
    if (!openLogsFor) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") closeLogs();
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [openLogsFor]);

  // A freshly opened panel always starts pinned to the newest line, with no stale "Copied".
  useEffect(() => {
    setAutoscroll(true);
    setCopied(false);
  }, [openLogsFor]);

  // Autoscroll — suspended as soon as the user scrolls up, resumed when they scroll back down.
  useEffect(() => {
    if (!autoscroll) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines, autoscroll, openLogsFor]);

  if (!openLogsFor || !project) return null;

  return (
    <div className="fixed inset-0 z-20 flex justify-end" role="presentation">
      {/* Click-away closes, same as Esc. */}
      <div
        className="flex-1 bg-black/40"
        onClick={closeLogs}
        aria-hidden="true"
      />

      <aside
        role="dialog"
        aria-modal="true"
        aria-label={`Logs for ${project.name}`}
        className="flex h-full w-[min(46rem,92vw)] flex-col border-l border-white/10 bg-surface shadow-2xl"
      >
        <header className="flex items-center justify-between gap-3 border-b border-white/5 px-5 py-4">
          <div className="min-w-0">
            <h2 className="truncate font-display text-base font-medium text-text">
              {project.name}
            </h2>
            <p className="mt-0.5 font-mono text-xs text-muted">
              :{project.port} · {lines.length} line{lines.length === 1 ? "" : "s"}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              onClick={() => void handleCopy()}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              {copied ? "Copied" : "Copy"}
            </button>
            <button
              type="button"
              onClick={() => void clearLogs(project.id)}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              Clear
            </button>
            <button
              type="button"
              aria-label="Close logs"
              onClick={closeLogs}
              className="rounded-md border border-white/10 px-3 py-1.5 text-sm text-muted transition-colors hover:bg-white/5 hover:text-text"
            >
              <span aria-hidden="true">✕</span>
            </button>
          </div>
        </header>

        <div
          ref={scrollRef}
          onScroll={(event) => {
            const el = event.currentTarget;
            const atBottom =
              el.scrollHeight - el.scrollTop - el.clientHeight <= BOTTOM_SLACK_PX;
            setAutoscroll(atBottom);
          }}
          className="flex-1 overflow-y-auto px-5 py-4 font-mono text-xs leading-relaxed"
        >
          {lines.length === 0 ? (
            <p className="text-muted">No output yet.</p>
          ) : (
            lines.map((line, index) => (
              <p
                key={index}
                className={`whitespace-pre-wrap break-words ${STREAM_TONE[line.stream]}`}
              >
                {line.line}
              </p>
            ))
          )}
        </div>

        {!autoscroll && (
          <button
            type="button"
            onClick={() => setAutoscroll(true)}
            className="border-t border-white/5 px-5 py-2 text-left text-xs text-muted transition-colors hover:bg-white/5 hover:text-text"
          >
            Autoscroll paused — jump to the newest line
          </button>
        )}
      </aside>
    </div>
  );
}

export default LogPanel;
