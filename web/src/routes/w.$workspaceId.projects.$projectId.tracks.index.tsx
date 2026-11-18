import { createFileRoute } from "@tanstack/react-router";
import { TracksPage } from "@/components/project/tracks-list";

// The project's Tracks, grouped by state, with a filter on state, owner and part.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/")({
  staticData: { section: "tracks" },
  component: Tracks,
});

function Tracks() {
  const { workspaceId, projectId } = Route.useParams();
  return <TracksPage workspaceId={workspaceId} projectId={projectId} />;
}
