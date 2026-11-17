import { createFileRoute, redirect } from "@tanstack/react-router";

// The old Threads page: narrowed to a track (`scope`), it is that track's
// Threads tab now; otherwise the project's Overview.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/threads/")({
  validateSearch: (search: Record<string, unknown>): { scope?: string } => ({
    scope: typeof search.scope === "string" && search.scope ? search.scope : undefined,
  }),
  beforeLoad: ({ params, search }) => {
    if (search.scope && search.scope !== "app") {
      throw redirect({ to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/threads", params: { ...params, trackId: search.scope }, replace: true });
    }
    throw redirect({ to: "/w/$workspaceId/projects/$projectId", params, replace: true });
  },
});
