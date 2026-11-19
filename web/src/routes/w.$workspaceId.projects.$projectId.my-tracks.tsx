import { createFileRoute, redirect } from "@tanstack/react-router";

// A project's old My tracks is Home's My tracks now.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/my-tracks")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: "/w/$workspaceId/my-tracks", params: { workspaceId: params.workspaceId }, replace: true });
  },
});
