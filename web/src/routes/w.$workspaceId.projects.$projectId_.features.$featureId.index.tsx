import { createFileRoute } from "@tanstack/react-router";
import { ScopePage } from "@/components/project/scope-page";

// A feature's scope page: the same page as the App's, over the feature's docs and threads.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/$featureId/")({
  component: FeatureScope,
});

function FeatureScope() {
  const { workspaceId, projectId, featureId } = Route.useParams();
  return <ScopePage workspaceId={workspaceId} projectId={projectId} featureId={featureId} />;
}
