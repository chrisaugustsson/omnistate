import { createFileRoute } from "@tanstack/react-router";
import { TrackThreads } from "@/components/track/track-threads";

// A track's Threads tab: its threads, open and concluded, each opening in place.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/threads/")({
  staticData: { section: "threads" },
  component: Threads,
});

function Threads() {
  return <TrackThreads />;
}
