import { createFileRoute } from "@tanstack/react-router";
import { ChangesetsPage } from "@/components/project/section-pages";

// The project's own change sets; each feature's are on the feature's page.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/changesets/")({
  component: Page,
});

function Page() {
  const { workspaceId, projectId } = Route.useParams();
  return <ChangesetsPage workspaceId={workspaceId} projectId={projectId} />;
}
