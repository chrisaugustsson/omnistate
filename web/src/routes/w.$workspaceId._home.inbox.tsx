import { createFileRoute } from "@tanstack/react-router";
import { Inbox } from "@/components/home/inbox";

// Home's Inbox: the comments waiting on you, as a list and the open one
// (components/home/inbox.tsx). `?comment=` is the open comment.
export const Route = createFileRoute("/w/$workspaceId/_home/inbox")({
  staticData: { tab: "inbox" },
  validateSearch: (search: Record<string, unknown>): { comment?: string } =>
    typeof search.comment === "string" && search.comment ? { comment: search.comment } : {},
  component: Page,
});

function Page() {
  const { workspaceId } = Route.useParams();
  const { comment } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <Inbox
      workspaceId={workspaceId}
      selected={comment}
      onSelect={(id) => void navigate({ search: { comment: id }, replace: true })}
    />
  );
}
