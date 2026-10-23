import { createFileRoute } from "@tanstack/react-router";
import { ChangesetsPage } from "@/components/project/section-pages";

// A feature's own change sets.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/$featureId/changesets/")({
  component: Page,
});

function Page() {
  const { workspaceId, projectId, featureId } = Route.useParams();
  return <ChangesetsPage workspaceId={workspaceId} projectId={projectId} featureId={featureId} />;
}
