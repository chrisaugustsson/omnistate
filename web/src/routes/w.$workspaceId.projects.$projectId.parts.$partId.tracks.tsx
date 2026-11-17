import { createFileRoute } from "@tanstack/react-router";
import { PlaceholderPage } from "@/components/shell/page";

// A part's Tracks: its tracks grouped by state. Slice S2 builds it.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/tracks")({
  staticData: { tab: "tracks" },
  component: Tracks,
});

function Tracks() {
  return <PlaceholderPage title="Tracks">The part&rsquo;s tracks, grouped by state.</PlaceholderPage>;
}
