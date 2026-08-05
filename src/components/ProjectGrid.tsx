/**
 * The card grid — SPEC.md §11.
 * Cards render in `projects.json` array order. No sorting, ever.
 */
import type { ProjectView } from "../types";
import ProjectCard from "./ProjectCard";

export function ProjectGrid({ projects }: { projects: ProjectView[] }) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(20rem,1fr))] gap-4">
      {projects.map((project) => (
        <ProjectCard key={project.id} project={project} />
      ))}
    </div>
  );
}

export default ProjectGrid;
