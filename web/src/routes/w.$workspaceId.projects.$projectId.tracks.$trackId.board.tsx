import { createFileRoute } from "@tanstack/react-router";
import { useIsPhone } from "@/components/shell/shell-context";
import { Board } from "@/components/stories/board";

// A track's story board: its stories by status, one card each. On a desktop it
// is a canvas, its columns filling the room beside the folded sidebar; on the
// phone it is one list grouped by status, in the page's column.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/board")({
  staticData: { section: "board", canvas: true },
  component: BoardRoute,
});

function BoardRoute() {
  const { workspaceId, projectId, trackId } = Route.useParams();
  const phone = useIsPhone();
  if (phone) return <Board workspaceId={workspaceId} projectId={projectId} trackId={trackId} list />;
  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-4 pt-3 pb-8">
      <Board workspaceId={workspaceId} projectId={projectId} trackId={trackId} />
    </div>
  );
}
