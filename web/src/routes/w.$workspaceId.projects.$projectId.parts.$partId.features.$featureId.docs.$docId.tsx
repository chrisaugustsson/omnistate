import { createFileRoute } from "@tanstack/react-router";
import { FeatureDocs } from "@/components/track/feature-docs";
import { useFeature } from "@/components/track/feature-data";
import { validateDocSearch } from "@/components/viewer/doc-page";

// A doc on a feature, open in the feature's Docs tab beside the list.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/docs/$docId")({
  staticData: { section: "docs" },
  validateSearch: validateDocSearch,
  component: FeatureDoc,
});

function FeatureDoc() {
  const { docId } = Route.useParams();
  const { docs } = useFeature();
  return (
    <FeatureDocs
      docs={docs}
      current={docId}
      search={Route.useSearch()}
      empty={{ title: "No docs yet", body: "Docs written about this feature (how it works, its tables) show here." }}
    />
  );
}
