import { createFileRoute } from "@tanstack/react-router";
import { ScopePage } from "@/components/project/scope-page";

// A track's scope page: the same page as the App's, over the track's docs and threads.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/tracks/$trackId/")({
  component: TrackScope,
});

function TrackScope() {
  const { workspaceId, projectId, trackId } = Route.useParams();
  return <ScopePage workspaceId={workspaceId} projectId={projectId} trackId={trackId} />;
}
