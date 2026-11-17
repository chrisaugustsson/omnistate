import { useCallback } from "react";
import { loadMyUserId } from "@/api/members";
import { PageHeader } from "@/components/page-header";
import { Empty, List } from "@/components/project/items";
import { TrackRow, orderTracks } from "@/components/project/track-row";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useReloading } from "@/lib/reloading";
import { useHome, useProjectName } from "@/lib/use-home";

// My tracks of one project: the tracks the signed-in member owns, across
// parts, active ones first. Each row opens the track page. Home's My tracks
// shows it for each project of the workspace until S4 builds Home.
export function ProjectMyTracks({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { home, failed } = useHome(workspaceId, projectId);
  const loadMe = useCallback(() => loadMyUserId(workspaceId), [workspaceId]);
  const { value: me, failed: meFailed } = useReloading(loadMe);
  const name = useProjectName(workspaceId, projectId);
  const mine =
    home && me ? orderTracks((home.tracks ?? []).filter(({ track }) => track.owner_user_id === me)) : null;

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader eyebrow={name ?? "Project"} title="My tracks" description="The tracks you own in this project, active ones first." />
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
