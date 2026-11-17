import { createFileRoute } from "@tanstack/react-router";
import { ScopePage } from "@/components/project/scope-page";

// A track's Spec tab. Until S3 builds it, the track's scope page as it was:
// the same page as the project Overview's, over the track's docs and threads.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/")({
  component: TrackScope,
});

function TrackScope() {
  const { workspaceId, projectId, trackId } = Route.useParams();
  return <ScopePage workspaceId={workspaceId} projectId={projectId} trackId={trackId} />;
}
