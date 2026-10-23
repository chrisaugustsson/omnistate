import { createFileRoute } from "@tanstack/react-router";
import { ThreadsPage } from "@/components/project/section-pages";

// The project's own threads; each feature's are on the feature's page.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/threads/")({
  component: Page,
});

function Page() {
  const { workspaceId, projectId } = Route.useParams();
  return <ThreadsPage workspaceId={workspaceId} projectId={projectId} />;
}
