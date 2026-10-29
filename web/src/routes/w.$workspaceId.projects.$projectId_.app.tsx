import { createFileRoute } from "@tanstack/react-router";
import { ScopePage } from "@/components/project/scope-page";

// The App scope page: the project's app level, the app as intended now.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/app")({
  component: AppScope,
});

function AppScope() {
  const { workspaceId, projectId } = Route.useParams();
  return <ScopePage workspaceId={workspaceId} projectId={projectId} featureId={null} />;
}
