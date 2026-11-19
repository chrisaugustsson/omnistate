import { createFileRoute } from "@tanstack/react-router";
import { FeatureDocs } from "@/components/track/feature-docs";
import { useFeature } from "@/components/track/feature-data";
import { validateDocSearch } from "@/components/viewer/doc-page";

// A feature's Docs: the docs on the feature beside the open doc, the first one open.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/docs/")({
  staticData: { section: "docs" },
  validateSearch: validateDocSearch,
  component: Docs,
});

function Docs() {
  const { docs } = useFeature();
  return (
    <FeatureDocs
      docs={docs}
      search={Route.useSearch()}
      empty={{ title: "No docs yet", body: "Docs written about this feature (how it works, its tables) show here." }}
    />
  );
}
