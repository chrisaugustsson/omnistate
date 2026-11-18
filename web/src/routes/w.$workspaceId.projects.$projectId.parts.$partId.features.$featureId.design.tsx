import { useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import { FeatureDocs } from "@/components/track/feature-docs";
import { useFeature } from "@/components/track/feature-data";
import { validateDocSearch } from "@/components/viewer/doc-page";

// A feature's Design: its design docs (UI pillar, boards and the design system) in the viewer.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/design")({
  staticData: { section: "design", canvas: true },
  validateSearch: validateDocSearch,
  component: Design,
});

function Design() {
  const { designDocs } = useFeature();
  const [open, setOpen] = useState<string | undefined>(undefined);
  return (
    <FeatureDocs
      docs={designDocs}
      current={open}
      onPick={setOpen}
      search={Route.useSearch()}
      empty={{ title: "No design yet", body: "The feature's screens arrive with the design boards of a track that changes it." }}
    />
  );
}
