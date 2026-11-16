import { useCallback } from "react";
import { createFileRoute } from "@tanstack/react-router";
import { loadMyUserId } from "@/api/members";
import { PageHeader } from "@/components/page-header";
import { Empty, List } from "@/components/project/items";
import { TrackRow, orderTracks } from "@/components/project/track-row";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useReloading } from "@/lib/reloading";
import { useHome } from "@/lib/use-home";

// My tracks: the tracks of the project the signed-in member owns, across
// parts, active ones first. Each row opens the track page.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/my-tracks")({
  component: MyTracks,
});

function MyTracks() {
  const { workspaceId, projectId } = Route.useParams();
  const { home, failed } = useHome(workspaceId, projectId);
  const loadMe = useCallback(() => loadMyUserId(workspaceId), [workspaceId]);
  const { value: me, failed: meFailed } = useReloading(loadMe);
  const mine =
    home && me ? orderTracks((home.tracks ?? []).filter(({ track }) => track.owner_user_id === me)) : null;

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader title="My tracks" description="The tracks you own in this project, active ones first." />
      {(failed || meFailed) && (
        <Alert variant="destructive" className="mt-6">
          <AlertDescription>The page could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {mine && (
        <div className="mt-6">
          {mine.length === 0 ? (
            <Empty>You own no tracks in this project</Empty>
          ) : (
            <List>
              {mine.map((summary) => (
                <TrackRow key={summary.track.id} summary={summary} params={{ workspaceId, projectId }} showPart />
              ))}
            </List>
          )}
        </div>
      )}
    </main>
  );
}
