import { createFileRoute } from "@tanstack/react-router";
import { Board } from "@/components/stories/board";

// A track's story board: its stories by status, one card each.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/board")({
  staticData: { section: "board", canvas: true },
  component: BoardRoute,
});

function BoardRoute() {
  const { workspaceId, projectId, trackId } = Route.useParams();
  return <Board workspaceId={workspaceId} projectId={projectId} trackId={trackId} />;
}
