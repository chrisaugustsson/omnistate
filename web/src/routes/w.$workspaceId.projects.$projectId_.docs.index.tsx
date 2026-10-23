import { createFileRoute } from "@tanstack/react-router";
import { DocsPage } from "@/components/project/section-pages";

// The project's own docs; each feature's are on the feature's page.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/docs/")({
  component: Page,
});

function Page() {
  const { workspaceId, projectId } = Route.useParams();
  return <DocsPage workspaceId={workspaceId} projectId={projectId} />;
}
