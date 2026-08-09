/**
 * The card grid — SPEC.md §11.
 * Cards render in `projects.json` array order. No sorting, ever.
 *
 * The trailing `+` tile (plan 020) is not a project card: it renders after `projects.map(...)`,
 * outside it, so it can never affect card order. `App.tsx` only mounts this component when the
 * (possibly search-filtered) list is non-empty, which already keeps the tile off the true
 * first-run empty state — that state owns its own single Add affordance (SPEC.md §11).
 */
import { openAddDialog } from "../store";
import type { ProjectView } from "../types";
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

export function ProjectGrid({ projects }: { projects: ProjectView[] }) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-3">
      {projects.map((project) => (
        <ProjectCard key={project.id} project={project} />
      ))}
      <AddTile />
    </div>
  );
}

export default ProjectGrid;
