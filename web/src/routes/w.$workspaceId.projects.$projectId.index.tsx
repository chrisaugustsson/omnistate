import { createFileRoute } from "@tanstack/react-router";
import { ScopePage } from "@/components/project/scope-page";

// The project's Overview. Until S2 builds it, the project's scope page as it
// was at /app: the project's name, its parts, decisions and project-level docs.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/")({
  staticData: { tab: "overview" },
  component: ProjectOverview,
});

function ProjectOverview() {
  const { workspaceId, projectId } = Route.useParams();
  return <ScopePage workspaceId={workspaceId} projectId={projectId} trackId={null} />;
}
