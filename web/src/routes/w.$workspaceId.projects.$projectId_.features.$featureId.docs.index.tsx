import { createFileRoute } from "@tanstack/react-router";
import { DocsPage } from "@/components/project/section-pages";

// A feature's own docs.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/$featureId/docs/")({
  component: Page,
});

function Page() {
  const { workspaceId, projectId, featureId } = Route.useParams();
  return <DocsPage workspaceId={workspaceId} projectId={projectId} featureId={featureId} />;
}
