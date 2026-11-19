import { createFileRoute } from "@tanstack/react-router";
import { ScopeDocs } from "@/components/project/docs-pane";

// A part's Docs with no doc open: the list of the part's and its features'
// docs, and the most recently updated one opens (or the tab says there are none).
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/docs/")({
  staticData: { section: "docs", bleed: true },
  component: Docs,
});

function Docs() {
  const { workspaceId, projectId, partId } = Route.useParams();
  return <ScopeDocs workspaceId={workspaceId} projectId={projectId} partId={partId} />;
}
