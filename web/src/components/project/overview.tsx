import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { describeProject } from "@/api/home";
import { describePart } from "@/api/parts";
import { PageHead } from "@/components/shell/page";
import { projectLink } from "@/components/shell/scope";
import { HeaderActions, useIsPhone } from "@/components/shell/shell-context";
import { useProjects } from "@/lib/use-workspace";
import { Intro } from "./intro";
import { ProjectMenu, ProjectTitle } from "./project-title";
import { useProjectMap } from "./project-data";
import { ScopePage } from "./rows";

// The Overview of a project and of a part is its README (navigation F): the
// title, then the description as page text, edited in place, or a quiet "Add a
// description" while there is none. Parts, tracks, docs and decisions each
// have their own section in the sidebar, whose counts are the only numbers.
// On the phone the header names the project or part, so the title is there
// for screen readers only, and the project's "…" menu sits in the header.

type Params = { workspaceId: string; projectId: string };

/** The page could not be loaded: one quiet line. */
function Failed() {
  return (
    <p role="alert" className="mt-4 rounded-control bg-danger-tint px-3 py-2 text-danger-ink">
      Part of the page could not be loaded. Try again in a moment.
    </p>
  );
}

/** A project's Overview. */
export function ProjectOverview({ workspaceId, projectId }: Params) {
  const { projects, failed, reload } = useProjects(workspaceId, projectId);
  const project = projects?.find((candidate) => candidate.id === projectId);
  const [renaming, setRenaming] = useState(false);
  const phone = useIsPhone();

  return (
    <ScopePage>
      <div className="max-w-[680px]">
        <div className="flex min-h-(--control-h-lg) items-center gap-3 max-md:min-h-0">
          {project ? (
            <ProjectTitle
              workspaceId={workspaceId}
              projectId={projectId}
              name={project.name}
              editing={renaming}
              onEditing={setRenaming}
              className="max-md:sr-only"
            />
          ) : (
            <span aria-hidden className="block h-6 w-56 animate-pulse rounded-control bg-hover" />
          )}
          {project && !renaming && !phone && (
            <div className="ml-auto flex shrink-0 items-center gap-2">
              <ProjectMenu onRename={() => setRenaming(true)} />
            </div>
          )}
          {project && !renaming && phone && (
            <HeaderActions>
              <ProjectMenu onRename={() => setRenaming(true)} />
            </HeaderActions>
          )}
        </div>
        {failed && <Failed />}
        {project && (
          <Intro
            className="max-md:mt-0"
            text={project.description}
            onSave={async (text) => {
              await describeProject(workspaceId, projectId, text);
              reload();
            }}
          />
        )}
      </div>
    </ScopePage>
  );
}

/** A part's Overview. */
export function PartOverview({ workspaceId, projectId, partId }: Params & { partId: string }) {
  const params = { workspaceId, projectId };
  const { map, failed, reload } = useProjectMap(workspaceId, projectId);
  const part = map?.parts.find((candidate) => candidate.id === partId);
  if (map && !part) return <PartNotFound params={params} />;

  return (
    <ScopePage>
      <div className="max-w-[680px]">
        <PageHead title={part?.name ?? " "} className="max-md:sr-only" />
        {failed && <Failed />}
        {part && (
          <Intro
            className="max-md:mt-0"
            text={part.description}
            onSave={async (text) => {
              await describePart(workspaceId, projectId, partId, text);
              reload();
            }}
          />
        )}
      </div>
    </ScopePage>
  );
}

/** A part that is not there (deleted, or of another project). */
export function PartNotFound({ params }: { params: Params }) {
  return (
    <ScopePage>
      <PageHead title="Part not found" />
      <p className="mt-2 text-em text-ink-2">
        The project has no such part, or it was deleted.{" "}
        <Link {...projectLink(params.workspaceId, params.projectId, "parts")} className="font-medium text-ink underline underline-offset-2">
          See the project&rsquo;s parts
        </Link>
      </p>
    </ScopePage>
  );
}
