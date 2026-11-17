import { createFileRoute } from "@tanstack/react-router";
import { TrackDocTab } from "@/components/track/track-docs";
import { validateDocSearch } from "@/components/viewer/doc-page";

// A track's How tab: how it is built (its architecture and tables docs) in the doc viewer.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/how")({
  validateSearch: validateDocSearch,
  component: How,
});

function How() {
  return <TrackDocTab tab="how" search={Route.useSearch()} />;
}
