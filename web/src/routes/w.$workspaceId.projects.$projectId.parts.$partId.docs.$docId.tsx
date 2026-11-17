import { createFileRoute } from "@tanstack/react-router";
import { DocPage, validateDocSearch } from "@/components/viewer/doc-page";

// A doc on a part, open in the part's Docs tab.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/docs/$docId")({
  staticData: { tab: "docs", bleed: true },
  validateSearch: validateDocSearch,
  component: PartDoc,
});

function PartDoc() {
  const { workspaceId, projectId, docId } = Route.useParams();
  return <DocPage workspaceId={workspaceId} projectId={projectId} docId={docId} search={Route.useSearch()} />;
}
