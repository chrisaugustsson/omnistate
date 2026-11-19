import { createFileRoute } from "@tanstack/react-router";
import { TrackThreads } from "@/components/track/track-threads";

// A thread of the track, open in its Threads tab under the list: followed live, with the form to post.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/threads/$threadId")({
  staticData: { section: "threads" },
  component: Thread,
});

function Thread() {
  const { threadId } = Route.useParams();
  return <TrackThreads threadId={threadId} />;
}
