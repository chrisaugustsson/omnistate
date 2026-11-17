import { createFileRoute } from "@tanstack/react-router";
import { TrackDecisions } from "@/components/project/decisions";
import { Page } from "@/components/shell/page";
import { useHome } from "@/lib/use-home";

// A track's Decisions. Until S3 builds the log, the track's decisions section as
// it was on the track page, where the owner records one.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/decisions")({
  component: Decisions,
});

function Decisions() {
  const { workspaceId, projectId, trackId } = Route.useParams();
  const { home } = useHome(workspaceId, projectId);
  const owner = home?.tracks?.find((summary) => summary.track.id === trackId)?.track.owner_user_id ?? null;
  return (
    <Page object>
      <TrackDecisions workspaceId={workspaceId} projectId={projectId} trackId={trackId} ownerUserId={owner} />
    </Page>
  );
}
