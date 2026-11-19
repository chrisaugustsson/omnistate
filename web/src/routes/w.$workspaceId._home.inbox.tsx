import { createFileRoute, redirect } from "@tanstack/react-router";

// The Inbox is the workspace's own page now (/w/<workspace>); its old URL leads
// there, keeping the comment to open.
export const Route = createFileRoute("/w/$workspaceId/_home/inbox")({
  validateSearch: (search: Record<string, unknown>): { comment?: string } =>
    typeof search.comment === "string" && search.comment ? { comment: search.comment } : {},
  beforeLoad: ({ params, search }) => {
    throw redirect({ to: "/w/$workspaceId", params: { workspaceId: params.workspaceId }, search, replace: true });
  },
});
