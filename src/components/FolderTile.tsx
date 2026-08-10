/**
 * The folder tile — SPEC.md §11 "Folders". A second kind of grid tile, not a card: shows exactly
 * four things — the name, a member count, a row of per-member dots, and an aggregate line. No
 * status pill, no port, no stack badge, no libraries line, no phase strip, no Run/Stop button —
 * §6's status vocabulary belongs to projects; a folder has no state machine.
 *
 * Root is an `<article>`, never a `<button>` — it hosts the `⋯` menu button and, while renaming,
 * a text `<input>`, and a button may not contain either. Open/close is instead a stretched
 * `absolute inset-0` button underneath everything else, so the whole tile is one click target
 * while `⋯` and the rename input remain independently interactive siblings at `relative z-10` —
 * the same layering `ProjectCard.tsx`'s own `⋯` menu already relies on.
 */
import { useEffect, useRef, useState } from "react";
import { folderSummary, renameFolder, toggleFolder, ungroupFolder } from "../store";
import { STATUS_LABEL, STATUS_TONE } from "../status";
import type { ProjectView } from "../types";

/** §11: "capped at eight then +n". */
const DOT_CAP = 8;

function FolderDots({ members }: { members: ProjectView[] }) {
  const shown = members.slice(0, DOT_CAP);
  const overflow = members.length - shown.length;
  return (
    <div className="pointer-events-none relative z-10 flex items-center gap-1">
      <div aria-hidden="true" className="flex items-center gap-1">
        {shown.map((m) => (
          <span
            key={m.id}
            className={`size-1.5 rounded-full ${STATUS_TONE[m.status]} ${
              m.pathExists ? "bg-current" : "border border-current bg-transparent"
            }`}
          />
        ))}
        {overflow > 0 && <span className="ml-0.5 text-xs text-muted">+{overflow}</span>}
      </div>
      {/* §11: the dot row is display-only; the real information is here for assistive tech. */}
      <ul className="sr-only">
        {members.map((m) => (
          <li key={m.id}>
            {m.name}: {STATUS_LABEL[m.status]}
            {m.pathExists ? "" : ", project folder not found"}
          </li>
        ))}
      </ul>
    </div>
  );
}

export function FolderTile({
  id,
  name,
  members,
  open,
}: {
  id: string;
  name: string;
  members: ProjectView[];
  /** Derived at render time by the caller (openFolders bit OR a stop-failed member) — this
   *  component never mutates that decision, only reflects it (§11: auto-expand-and-cannot-collapse
   *  is a render-time predicate, not a stored bit). */
  open: boolean;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [draftName, setDraftName] = useState(name);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  // Escape cancels without re-committing the value on the blur that follows it.
  const skipBlurCommit = useRef(false);

  useEffect(() => {
    if (!menuOpen) return;
    function onPointerDown(event: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    }
    document.addEventListener("mousedown", onPointerDown);
    return () => document.removeEventListener("mousedown", onPointerDown);
  }, [menuOpen]);

  useEffect(() => {
    if (renaming) inputRef.current?.focus();
  }, [renaming]);

  function startRename() {
    setMenuOpen(false);
    setDraftName(name);
    // §11: rename is inline in the open tile, never on the closed one — opening first avoids the
    // input colliding with the stretched open/close button underneath it.
    if (!open) toggleFolder(id);
    setRenaming(true);
  }

  function commitRename() {
    const trimmed = draftName.trim();
    setRenaming(false);
    // Empty/whitespace commit reverts — simply not writing leaves `name` (the prop) as-is.
    if (trimmed !== "" && trimmed !== name) void renameFolder(id, trimmed);
  }

  function handleInputBlur() {
    if (skipBlurCommit.current) {
      skipBlurCommit.current = false;
      return;
    }
    commitRename();
  }

  async function handleUngroup() {
    setMenuOpen(false);
    // §11: the word "Remove" must never appear on a folder — that means "destroy this project,
    // cannot be undone" elsewhere in this app. Ungroup only ever dissolves the grouping.
    if (window.confirm(`Ungroup "${name}"? The ${members.length} projects stay in your library.`)) {
      await ungroupFolder(id);
    }
  }

  const bandId = `folder-band-${id}`;

  return (
    <article
      className="hangar-fade-in relative flex min-h-[11rem] cursor-pointer flex-col gap-2 rounded-lg border border-white/10 bg-surface p-3 before:absolute before:inset-x-3 before:-top-1 before:h-px before:bg-white/10 before:content-[''] after:absolute after:inset-x-5 after:-top-2 after:h-px after:bg-white/5 after:content-['']"
    >
      <button
        type="button"
        aria-expanded={open}
        aria-controls={bandId}
        onClick={() => toggleFolder(id)}
        className="absolute inset-0 z-0 rounded-lg"
      >
        <span className="sr-only">
          {open ? "Collapse" : "Expand"} folder {name}
        </span>
      </button>

      {/* Display-only by default — pointer-events-none lets clicks fall through to the stretched
          open/close button beneath. The `⋯` menu and the name each punch a pointer-events-auto
          hole back open below, since they're independently interactive. */}
      <header className="pointer-events-none relative z-10 flex items-start justify-between gap-3">
        <div
          className={`pointer-events-auto flex min-w-0 items-center gap-1.5 ${
            renaming ? "" : "cursor-pointer"
          }`}
          onClick={renaming ? undefined : () => toggleFolder(id)}
        >
          <span aria-hidden="true" className="shrink-0 text-muted">
            {open ? "⌄" : "›"}
          </span>
          {renaming ? (
            <input
              ref={inputRef}
              type="text"
              value={draftName}
              onChange={(e) => setDraftName(e.target.value)}
              onBlur={handleInputBlur}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
                else if (e.key === "Escape") {
                  skipBlurCommit.current = true;
                  setDraftName(name);
                  setRenaming(false);
                }
              }}
              className="pointer-events-auto min-w-0 flex-1 rounded border border-accent bg-bg px-1.5 py-0.5 font-display text-lg font-bold tracking-tight text-text outline-none"
            />
          ) : (
            <h2
              className="truncate font-display text-lg font-bold tracking-tight text-text"
              title={name}
            >
              {name}
            </h2>
          )}
        </div>

        <div className="pointer-events-auto relative shrink-0" ref={menuRef}>
          <button
            type="button"
            aria-label={`Actions for folder ${name}`}
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            onClick={() => setMenuOpen((v) => !v)}
            className="rounded px-2 py-1 text-muted transition-colors hover:bg-white/5 hover:text-text"
          >
            <span aria-hidden="true">⋯</span>
          </button>
          {menuOpen && (
            <div
              role="menu"
              className="absolute right-0 z-10 mt-1 w-40 overflow-hidden rounded-md border border-white/10 bg-bg py-1 shadow-lg"
            >
              <button
                role="menuitem"
                type="button"
                onClick={startRename}
                className="block w-full px-3 py-1.5 text-left text-sm text-text transition-colors hover:bg-white/5"
              >
                Rename
              </button>
              <button
                role="menuitem"
                type="button"
                onClick={() => void handleUngroup()}
                className="block w-full px-3 py-1.5 text-left text-sm text-text transition-colors hover:bg-white/5"
              >
                Ungroup
              </button>
            </div>
          )}
        </div>
      </header>

      {/* Display-only — pointer-events-none so clicks fall through to the stretched button. */}
      <p className="pointer-events-none relative z-10 text-xs text-muted">
        {members.length} project{members.length === 1 ? "" : "s"}
      </p>

      <FolderDots members={members} />

      <p className="pointer-events-none relative z-10 mt-auto text-xs text-muted">
        {folderSummary(members)}
      </p>
    </article>
  );
}

export default FolderTile;

