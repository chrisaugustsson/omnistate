import { createFileRoute } from "@tanstack/react-router";
import { MyTracks } from "@/components/home/my-tracks";

// Home's My tracks: the tracks you own across the workspace, by state
// (components/home/my-tracks.tsx).
export const Route = createFileRoute("/w/$workspaceId/_home/my-tracks")({
  staticData: { section: "my-tracks" },
  component: Page,
});

function Page() {
  const { workspaceId } = Route.useParams();
  return <MyTracks workspaceId={workspaceId} />;
}
