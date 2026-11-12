import { createFileRoute } from "@tanstack/react-router";
import { BehaviourScreen } from "@/components/behaviour/screen";

// A feature's behaviour tree: folders, graph and details of the chosen node.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/$featureId/behaviour")({
  component: BehaviourRoute,
});

function BehaviourRoute() {
  const { workspaceId, projectId, featureId } = Route.useParams();
  return <BehaviourScreen workspaceId={workspaceId} projectId={projectId} featureId={featureId} />;
}
