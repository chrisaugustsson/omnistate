import { createFileRoute, redirect } from "@tanstack/react-router";

// A project's old Inbox is Home's Inbox now.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/inbox")({
  beforeLoad: ({ params }) => {
    throw redirect({ to: "/w/$workspaceId/inbox", params: { workspaceId: params.workspaceId }, replace: true });
  },
});
