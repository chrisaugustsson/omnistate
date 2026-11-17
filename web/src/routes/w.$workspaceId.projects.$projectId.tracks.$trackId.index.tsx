import { createFileRoute } from "@tanstack/react-router";
import { TrackDocTab } from "@/components/track/track-docs";
import { validateDocSearch } from "@/components/viewer/doc-page";

// A track's Spec tab: its spec in the doc viewer, or who writes it while it has none.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/")({
  validateSearch: validateDocSearch,
  component: Spec,
});

function Spec() {
  return <TrackDocTab tab="spec" search={Route.useSearch()} />;
}
