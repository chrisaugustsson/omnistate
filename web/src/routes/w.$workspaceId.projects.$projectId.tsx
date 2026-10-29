import { createFileRoute, redirect } from "@tanstack/react-router";

// A project opens at its App scope page (V26).
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: "/w/$workspaceId/projects/$projectId/app", params, replace: true });
  },
});
