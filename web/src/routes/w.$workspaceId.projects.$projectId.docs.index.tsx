import { createFileRoute } from "@tanstack/react-router";
import { PlaceholderPage } from "@/components/shell/page";

// The project's Docs: the project-level docs grouped by pillar beside the open
// doc. Slice S2 builds it; a doc opens at docs/$docId.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/docs/")({
  staticData: { tab: "docs" },
  component: Docs,
});

function Docs() {
  return <PlaceholderPage title="Docs">The project&rsquo;s own docs, grouped by pillar, beside the open doc.</PlaceholderPage>;
}
