import { createFileRoute } from "@tanstack/react-router";
import { ThreadsPage } from "@/components/thread/threads-page";

// The project's threads of every scope; `scope` (`app` or a feature's ID) narrows them.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/threads/")({
  component: Page,
  validateSearch: (search: Record<string, unknown>): { scope?: string } => ({
    scope: typeof search.scope === "string" && search.scope ? search.scope : undefined,
  }),
});

function Page() {
  const { workspaceId, projectId } = Route.useParams();
  const { scope } = Route.useSearch();
  const navigate = Route.useNavigate();
  return (
    <ThreadsPage
      workspaceId={workspaceId}
      projectId={projectId}
      scope={scope}
      onScope={(next) => void navigate({ search: { scope: next } })}
    />
  );
}
