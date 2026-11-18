import { createFileRoute } from "@tanstack/react-router";
import { DecisionsPage } from "@/components/project/decisions-page";

// The project's Decisions: the decision log of the project, its parts and their
// features, grouped by what each decision is about.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/decisions")({
  staticData: { section: "decisions" },
  component: Decisions,
});

function Decisions() {
  const { workspaceId, projectId } = Route.useParams();
  return <DecisionsPage workspaceId={workspaceId} projectId={projectId} />;
}
