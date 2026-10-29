import { createFileRoute, redirect } from "@tanstack/react-router";

// A feature's threads are the Threads page narrowed to the feature.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/$featureId/threads/")({
  beforeLoad: ({ params }) => {
    const { workspaceId, projectId, featureId } = params;
    throw redirect({
      to: "/w/$workspaceId/projects/$projectId/threads",
      params: { workspaceId, projectId },
      search: { scope: featureId },
      replace: true,
    });
  },
});
