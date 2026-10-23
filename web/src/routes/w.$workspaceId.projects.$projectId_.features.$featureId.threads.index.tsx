import { createFileRoute } from "@tanstack/react-router";
import { ThreadsPage } from "@/components/project/section-pages";

// A feature's own threads.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/$featureId/threads/")({
  component: Page,
});

function Page() {
  const { workspaceId, projectId, featureId } = Route.useParams();
  return <ThreadsPage workspaceId={workspaceId} projectId={projectId} featureId={featureId} />;
}
