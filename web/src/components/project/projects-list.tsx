import { Link } from "@tanstack/react-router";
import type { PartBody, ProjectBody } from "@/api/generated";
import { PartIcon, ProjectMark } from "@/components/shell/part-icon";
import { inFlight } from "@/components/shell/state-icon";
import { projectLink } from "@/components/shell/scope";
import { partTone } from "@/lib/identity";
import { useProjects, useWorkspaceHomes, useWorkspaceParts } from "@/lib/use-workspace";
import { cn } from "@/lib/utils";
import { EmptyRow, rowClass, ScopePage, SectionBar } from "./rows";

// The workspace's Projects: a row per project (its mark, its name, its parts
// as small marks, and how many tracks are in flight), each opening the
// project's Overview.

const byName = (a: ProjectBody, b: ProjectBody) => a.name.localeCompare(b.name);

/** A project's parts as small marks in their colours, each named on hover. */
function PartMarks({ parts }: { parts: readonly PartBody[] }) {
  return (
    <span className="flex shrink-0 items-center gap-1" aria-label={parts.map((part) => part.name).join(", ")} role="img">
      {parts.map((part) => (
        <span key={part.id} title={`${part.name} · ${part.kind}`} className="inline-grid">
          <PartIcon name={part.name} kind={part.kind} tone={partTone(parts, part.id)} size="sm" />
        </span>
      ))}
    </span>
  );
}

/** The workspace's Projects page. */
export function ProjectsPage({ workspaceId }: { workspaceId: string }) {
  const { projects, failed } = useProjects(workspaceId);
  const { homes } = useWorkspaceHomes(workspaceId, projects);
  const { parts } = useWorkspaceParts(workspaceId, projects);
  const sorted = projects ? [...projects].sort(byName) : null;

  return (
    <ScopePage>
      <SectionBar title="Projects" />
      {failed && (
        <p role="alert" className="mt-4 text-danger-ink">
          The projects could not be loaded. Try again in a moment.
        </p>
      )}
      {sorted && (
        <ul aria-label="Projects" className="max-w-(--main-max) border-b border-line">
          {sorted.length === 0 ? (
            <EmptyRow>No projects yet. A project is a product, made of the apps and services that change together.</EmptyRow>
          ) : (
            sorted.map((project) => {
              const own = parts?.get(project.id);
              const tracks = homes?.get(project.id)?.tracks;
              const flying = tracks?.filter(({ track }) => inFlight(track.state)).length;
              return (
                <li key={project.id}>
                  <Link {...projectLink(workspaceId, project.id, "overview")} className={cn(rowClass({ link: true }), "py-2.5")}>
                    <ProjectMark name={project.name} parts={own?.map((part) => ({ kind: part.kind, tone: partTone(own, part.id) }))} />
                    <span className="min-w-0 flex-1 truncate font-medium">{project.name}</span>
                    {own && own.length > 0 && <PartMarks parts={own} />}
                    <span className="w-[86px] shrink-0 text-right text-meta whitespace-nowrap text-ink-3 tabular-nums max-sm:w-auto">
                      {flying === undefined ? "" : flying === 0 ? "None in flight" : `${flying} in flight`}
                    </span>
                  </Link>
                </li>
              );
            })
          )}
        </ul>
      )}
    </ScopePage>
  );
}
