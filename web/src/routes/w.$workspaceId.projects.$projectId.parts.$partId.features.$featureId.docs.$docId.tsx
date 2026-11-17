import { createFileRoute } from "@tanstack/react-router";
import { DocPage, validateDocSearch } from "@/components/viewer/doc-page";

// A doc on a feature, open in the feature's Docs tab.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/docs/$docId")({
  staticData: { bleed: true },
  validateSearch: validateDocSearch,
  component: FeatureDoc,
});

function FeatureDoc() {
  const { workspaceId, projectId, docId } = Route.useParams();
  return <DocPage workspaceId={workspaceId} projectId={projectId} docId={docId} search={Route.useSearch()} />;
}
