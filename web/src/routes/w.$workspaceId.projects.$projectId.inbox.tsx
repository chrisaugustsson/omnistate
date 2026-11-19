import { createFileRoute, redirect } from "@tanstack/react-router";

// A project's old Inbox is the workspace's Inbox now, its own page.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/inbox")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: "/w/$workspaceId", params: { workspaceId: params.workspaceId }, replace: true });
  },
});
