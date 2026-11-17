import { createFileRoute } from "@tanstack/react-router";
import { ThreadView } from "@/components/thread/thread-view";

// A thread of the track, open in its Threads tab: followed live, with the form to post.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/threads/$threadId")({
  component: Thread,
});

function Thread() {
  const { workspaceId, projectId, threadId } = Route.useParams();
  return <ThreadView workspaceId={workspaceId} projectId={projectId} threadId={threadId} />;
}
