import { createFileRoute } from "@tanstack/react-router";
import { BehaviourScreen } from "@/components/behaviour/screen";
import { useIsPhone } from "@/components/shell/shell-context";
import { BehaviourOutline } from "@/components/track/behaviour-outline";
import { useTrack } from "@/components/track/track-data";

// A track's Behaviour: on a desktop its behaviour tree (folders, graph and the
// chosen node's details) as a canvas filling the room beside the folded
// sidebar; on the phone the tree as an outline, a node opening full screen.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/behaviour")({
  staticData: { section: "behaviour", canvas: true },
  component: BehaviourRoute,
});

function BehaviourRoute() {
  const { user } = Route.useRouteContext();
  const { workspaceId, projectId, trackId } = Route.useParams();
  const { reportOpenQuestions } = useTrack();
  const phone = useIsPhone();
  if (phone) {
    return <BehaviourOutline workspaceId={workspaceId} projectId={projectId} trackId={trackId} me={user.id} onWaiting={reportOpenQuestions} />;
  }
  return (
    <div className="relative min-h-0 flex-1 overflow-hidden">
      <div className="absolute inset-0">
        <BehaviourScreen workspaceId={workspaceId} projectId={projectId} trackId={trackId} me={user.id} onWaiting={reportOpenQuestions} />
      </div>
    </div>
  );
}
