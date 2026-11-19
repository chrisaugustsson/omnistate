import { useHome } from "@/lib/use-home";
import { useProjects } from "@/lib/use-workspace";
import { DecisionLog } from "./decision-log";
import { PartNotFound } from "./overview";
import { useDecisionGroups, useMemberNames, useProjectMap } from "./project-data";
import { ScopePage, SectionBar } from "./rows";

// The Decisions of a project or a part: the decision log of the level and the
// levels under it, grouped by what each decision is about. Decisions are
// recorded on tracks, so the page has no way to record one.

type Params = { workspaceId: string; projectId: string };

/** The Decisions page of a project, or with `partId` of a part. */
export function DecisionsPage({ workspaceId, projectId, partId }: Params & { partId?: string }) {
  const params = { workspaceId, projectId };
  const { projects } = useProjects(workspaceId, projectId);
  const project = projects?.find((candidate) => candidate.id === projectId);
  const { home } = useHome(workspaceId, projectId);
  const { map } = useProjectMap(workspaceId, projectId);
  const { groups, failed } = useDecisionGroups(workspaceId, projectId, project?.name, map, partId);
  const names = useMemberNames(workspaceId);
  const part = partId ? map?.parts.find((candidate) => candidate.id === partId) : undefined;
  if (partId && map && !part) return <PartNotFound params={params} />;

  const name = partId ? part?.name : project?.name;
  return (
    <ScopePage>
      <SectionBar
        title="Decisions"
        note={
          partId
            ? `What was decided about ${name ?? "the part"} and its features, and why. A replaced decision folds under its successor.`
            : `What was decided in ${name ?? "the project"} and why, grouped by what each decision is about. A replaced decision folds under its successor.`
        }
      />
      {failed && (
        <p role="alert" className="mt-4 text-danger-ink">
          The decisions could not be loaded. Try again in a moment.
        </p>
      )}
      {groups && home && (
        <DecisionLog
          groups={groups}
          level={partId ? "part" : "project"}
          context={{ params, names, tracks: home.tracks ?? [], parts: map?.parts ?? null }}
          emptyText={`No decisions yet. A track records one when it settles a question; it shows here under what it is about.`}
        />
      )}
    </ScopePage>
  );
}
