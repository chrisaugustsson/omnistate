import { createFileRoute } from "@tanstack/react-router";
import { ThreadsPage } from "@/components/thread/threads-page";

// A track's Threads tab. Until S3 builds it, the old Threads page narrowed to
// the track; picking another scope goes to that track's tab.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/threads/")({
  component: Threads,
});

function Threads() {
  const { workspaceId, projectId, trackId } = Route.useParams();
  const navigate = Route.useNavigate();
  return (
    <ThreadsPage
      workspaceId={workspaceId}
      projectId={projectId}
      scope={trackId}
      onScope={(next) =>
        void (next
          ? navigate({ to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/threads", params: { workspaceId, projectId, trackId: next } })
          : navigate({ to: "/w/$workspaceId/projects/$projectId", params: { workspaceId, projectId } }))
      }
    />
  );
}
