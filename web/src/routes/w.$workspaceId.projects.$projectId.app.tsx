import { createFileRoute, redirect } from "@tanstack/react-router";

// The old App page is the project's Overview now.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/app")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: "/w/$workspaceId/projects/$projectId", params, replace: true });
  },
});
