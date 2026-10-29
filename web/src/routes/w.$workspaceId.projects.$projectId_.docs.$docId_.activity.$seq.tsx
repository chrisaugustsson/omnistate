import { createFileRoute } from "@tanstack/react-router";
import { ActivityEntryPage } from "@/components/activity/activity";

// One write to a doc, with what it changed before and after, side by side.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/docs/$docId_/activity/$seq")({
  component: Page,
});

function Page() {
  const { workspaceId, projectId, docId, seq } = Route.useParams();
  return <ActivityEntryPage workspaceId={workspaceId} projectId={projectId} docId={docId} seq={Number(seq)} />;
}
