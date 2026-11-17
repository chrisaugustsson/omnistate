import { createFileRoute } from "@tanstack/react-router";
import { BehaviourScreen } from "@/components/behaviour/screen";

// A track's behaviour tree: folders, graph and details of the chosen node.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/behaviour")({
  staticData: { bleed: true },
  component: BehaviourRoute,
});

function BehaviourRoute() {
  const { user } = Route.useRouteContext();
  const { workspaceId, projectId, trackId } = Route.useParams();
  return <BehaviourScreen workspaceId={workspaceId} projectId={projectId} trackId={trackId} me={user.id} />;
}
