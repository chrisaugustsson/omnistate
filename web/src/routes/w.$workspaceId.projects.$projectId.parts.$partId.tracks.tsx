import { createFileRoute } from "@tanstack/react-router";
import { TracksPage } from "@/components/project/tracks-list";

// A part's Tracks, grouped by state, with a filter on state and owner.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/tracks")({
  staticData: { section: "tracks" },
  component: Tracks,
});

function Tracks() {
  const { workspaceId, projectId, partId } = Route.useParams();
  return <TracksPage workspaceId={workspaceId} projectId={projectId} partId={partId} />;
}
