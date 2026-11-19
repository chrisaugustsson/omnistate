import { createFileRoute, redirect } from "@tanstack/react-router";
import { loadThread } from "@/api/threads";
import { ThreadView } from "@/components/thread/thread-view";

// The old URL of a thread. A track's thread opens in the track's Threads tab
// now; a thread of the project itself (no track) still opens here.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/threads/$threadId")({
  beforeLoad: async ({ params, location }) => {
    let trackId: string | null | undefined;
    try {
      trackId = (await loadThread(params.workspaceId, params.projectId, params.threadId)).track_id;
    } catch {
      return; // The page says the thread could not be loaded.
    }
    if (trackId) {
      throw redirect({
        to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/threads/$threadId",
        params: { ...params, trackId },
        hash: location.hash || undefined,
        replace: true,
      });
    }
  },
  component: ProjectThread,
});

function ProjectThread() {
  const { workspaceId, projectId, threadId } = Route.useParams();
  return <ThreadView workspaceId={workspaceId} projectId={projectId} threadId={threadId} />;
}
