import { createFileRoute } from "@tanstack/react-router";
import { DecisionsPage } from "@/components/project/decisions-page";

// A part's Decisions: the decision log of the part and its features.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/decisions")({
  staticData: { tab: "decisions" },
  component: Decisions,
});

function Decisions() {
  const { workspaceId, projectId, partId } = Route.useParams();
  return <DecisionsPage workspaceId={workspaceId} projectId={projectId} partId={partId} />;
}
