import { createFileRoute } from "@tanstack/react-router";
import { PartOverview } from "@/components/project/overview";

// A part's Overview: its name and description, its tracks in flight, its
// features, the latest decisions and docs about it, and the properties panel.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/")({
  staticData: { tab: "overview" },
  component: Overview,
});

function Overview() {
  const { workspaceId, projectId, partId } = Route.useParams();
  return <PartOverview workspaceId={workspaceId} projectId={projectId} partId={partId} />;
}
