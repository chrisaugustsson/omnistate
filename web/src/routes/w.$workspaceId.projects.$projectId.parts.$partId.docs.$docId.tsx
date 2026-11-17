import { createFileRoute } from "@tanstack/react-router";
import { ScopeDocs } from "@/components/project/docs-pane";
import { validateDocSearch } from "@/components/viewer/doc-page";

// A doc of a part (on the part or on one of its features), open in the part's
// Docs tab beside the list of the part's docs.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/docs/$docId")({
  staticData: { tab: "docs", bleed: true },
  validateSearch: validateDocSearch,
  component: PartDoc,
});

function PartDoc() {
  const { workspaceId, projectId, partId, docId } = Route.useParams();
  return <ScopeDocs workspaceId={workspaceId} projectId={projectId} partId={partId} docId={docId} search={Route.useSearch()} />;
}
