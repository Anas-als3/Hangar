/**
 * The card grid — SPEC.md §11 "Folders" / "Opening a folder" (plan 029 rewrite).
 *
 * Walks `gridItems(...)` instead of `projects` directly: a plain project renders as a card; the
 * first time the walk reaches a folder's member, a `FolderTile` is emitted in that array
 * position and — while open — a full-width band follows it as a sibling, holding the member
 * cards in a nested grid on the same track. No `sort`, no `concat` of two filtered lists, no
 * `grid-auto-flow: dense` (SPEC.md §11 forbids it by name — backfilling the hole an expanded
 * folder leaves would be re-sorting under another name).
 *
 * The trailing `+` tile (plan 020) is still not a grid item: it renders after the map, outside
 * it, so it can never affect item order. `App.tsx` only mounts this component when the (possibly
 * search-filtered) list is non-empty.
 */
import { Fragment } from "react";
import { closeFolder, gridItems, openAddDialog, useHangarStore } from "../store";
import type { ProjectView } from "../types";
import FolderTile from "./FolderTile";
import ProjectCard from "./ProjectCard";

function AddTile() {
  return (
    <button
      type="button"
      aria-label="Add project"
      onClick={openAddDialog}
      className="flex min-h-[8rem] flex-col items-center justify-center gap-1 rounded-lg border border-dashed border-white/10 p-3 text-muted transition-colors hover:border-white/20 hover:text-text"
    >
      <span aria-hidden="true" className="text-2xl leading-none">
        +
      </span>
      <span className="text-sm">Add project</span>
    </button>
  );
}

/** §11 "Opening a folder": a full-width band, immediately after its tile, member cards on the
 *  same track. Zero horizontal padding is load-bearing — with `p-3` the nested auto-fill drops a
 *  column at boundary widths and cards render *wider* inside a folder than outside it. */
function OpenBand({
  folderId,
  members,
}: {
  folderId: string;
  members: ProjectView[];
}) {
  const { openLogsFor, notesFor, dialog } = useHangarStore();
  return (
    <section
      id={`folder-band-${folderId}`}
      className="col-span-full grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-3 border-t border-b border-white/10 bg-white/[0.02] py-3"
      onKeyDown={(e) => {
        // §11/hard limit: Esc closes the band via the band's OWN onKeyDown, never a `document`
        // listener — a document listener would collide with ProjectCard's own menu Esc and the
        // search box's clear-on-Escape. Only fires while focus is inside this band.
        //
        // Plan 033 defect 1: the log panel, notes panel and dialogs (LogPanel.tsx, NotesPanel.tsx,
        // App.tsx's dialog host) are also Esc owners, each closing itself from its own `document`
        // listener that this band can't see or stop. A member card can be inside the band while
        // one of those is open, so the band must yield instead of also firing closeFolder — one
        // keypress must never trigger two unrelated state changes.
        if (e.key === "Escape" && !openLogsFor && !notesFor && !dialog) closeFolder(folderId);
      }}
    >
      {members.map((project) => (
        <ProjectCard key={project.id} project={project} />
      ))}
    </section>
  );
}

/** `search` drives whether `gridItems` dissolves folders (§11) — the caller passes the same
 *  string it already used for `visibleProjects`' empty-state check, so the two can never disagree. */
export function ProjectGrid({
  projects,
  search,
}: {
  projects: ProjectView[];
  search: string;
}) {
  const { openFolders } = useHangarStore();
  const items = gridItems(projects, search);

  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-3">
      {items.map((item) => {
        if (item.kind === "project") {
          return <ProjectCard key={item.project.id} project={item.project} />;
        }
        // §11: a folder auto-expands and cannot be collapsed while any member is `stop-failed` —
        // a derived predicate here, never a mutation of `openFolders`, so it can never get stuck.
        const open =
          openFolders.has(item.id) || item.members.some((m) => m.status === "stop-failed");
        return (
          <Fragment key={item.id}>
            <FolderTile id={item.id} name={item.name} members={item.members} open={open} />
            {open && <OpenBand folderId={item.id} members={item.members} />}
          </Fragment>
        );
      })}
      <AddTile />
    </div>
  );
}

export default ProjectGrid;

