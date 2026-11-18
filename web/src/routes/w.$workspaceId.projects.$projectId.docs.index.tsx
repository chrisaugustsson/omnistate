import { createFileRoute } from "@tanstack/react-router";
import { ScopeDocs } from "@/components/project/docs-pane";

// The project's Docs with no doc open: the list of the project's own docs, and
// the most recently updated one opens (or the tab says there are none).
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/docs/")({
  staticData: { section: "docs", bleed: true },
  component: Docs,
});

function Docs() {
  const { workspaceId, projectId } = Route.useParams();
  return <ScopeDocs workspaceId={workspaceId} projectId={projectId} />;
}
