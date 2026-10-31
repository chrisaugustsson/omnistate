import { createFileRoute } from "@tanstack/react-router";
import { Board } from "@/components/stories/board";

// A feature's story board: its stories by status, one card each.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/$featureId/board")({
  component: BoardRoute,
});

function BoardRoute() {
  const { workspaceId, projectId, featureId } = Route.useParams();
  return <Board workspaceId={workspaceId} projectId={projectId} featureId={featureId} />;
}
