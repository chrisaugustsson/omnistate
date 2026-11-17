import { createFileRoute } from "@tanstack/react-router";
import { ProjectOverview } from "@/components/project/overview";

// The project's Overview: its name and intro, the tracks in flight, the latest
// decisions and docs, and the properties panel with its parts and counts.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/")({
  staticData: { tab: "overview" },
  component: Overview,
});

function Overview() {
  const { workspaceId, projectId } = Route.useParams();
  return <ProjectOverview workspaceId={workspaceId} projectId={projectId} />;
}
