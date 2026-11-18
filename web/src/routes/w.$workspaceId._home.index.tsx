import { createFileRoute } from "@tanstack/react-router";
import { Inbox, validateInboxSearch, type InboxSearch } from "@/components/home/inbox";

// The workspace's own page is its Inbox: what waits on you, as a list with the
// open item beside it (components/home/inbox.tsx). `?comment=` or
// `?questions=<track>` is the open row, so a reload or Back keeps it.
export const Route = createFileRoute("/w/$workspaceId/_home/")({
  staticData: { section: "inbox" },
  validateSearch: validateInboxSearch,
  component: Page,
});

function Page() {
  const { workspaceId } = Route.useParams();
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <Inbox
      workspaceId={workspaceId}
      search={search}
      onSelect={(next: InboxSearch, replace = false) => void navigate({ search: next, replace })}
    />
  );
}
