import { createFileRoute } from "@tanstack/react-router";
import { PlaceholderPage } from "@/components/shell/page";

// A part's Docs: the docs on the part, grouped by pillar, beside the open doc.
// Slice S2 builds it; a doc opens at docs/$docId.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/docs/")({
  staticData: { tab: "docs" },
  component: Docs,
});

function Docs() {
  return <PlaceholderPage title="Docs">The part&rsquo;s docs, grouped by pillar, beside the open doc.</PlaceholderPage>;
}
