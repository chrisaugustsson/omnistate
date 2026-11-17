import { createFileRoute } from "@tanstack/react-router";
import { BehaviourScreen } from "@/components/behaviour/screen";
import { useTrack } from "@/components/track/track-data";

// A track's Behaviour tab: its behaviour tree (folders, graph and the chosen
// node's details) under the track's header, filling the room below the tabs.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/behaviour")({
  component: BehaviourRoute,
});

function BehaviourRoute() {
  const { user } = Route.useRouteContext();
  const { workspaceId, projectId, trackId } = Route.useParams();
  const { reportOpenQuestions } = useTrack();
  return (
    <div className="relative min-h-[480px] flex-1 overflow-hidden rounded-card border border-line">
      <div className="absolute inset-0">
        <BehaviourScreen workspaceId={workspaceId} projectId={projectId} trackId={trackId} me={user.id} onWaiting={reportOpenQuestions} />
      </div>
    </div>
  );
}
