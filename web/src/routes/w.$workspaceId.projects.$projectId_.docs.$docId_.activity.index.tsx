import { createFileRoute } from "@tanstack/react-router";
import { ActivityPage } from "@/components/activity/activity";

// A doc's Activity tab: the writes to it, newest first.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/docs/$docId_/activity/")({
  component: Page,
});

function Page() {
  const { workspaceId, projectId, docId } = Route.useParams();
  return <ActivityPage workspaceId={workspaceId} projectId={projectId} docId={docId} />;
}
