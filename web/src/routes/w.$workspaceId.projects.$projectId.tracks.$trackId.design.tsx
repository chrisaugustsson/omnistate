import { createFileRoute } from "@tanstack/react-router";
import { TrackDocTab } from "@/components/track/track-docs";
import { validateDocSearch } from "@/components/viewer/doc-page";

// A track's Design tab: its design boards (its UI docs) in the design viewer.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/design")({
  validateSearch: validateDocSearch,
  component: Design,
});

function Design() {
  return <TrackDocTab tab="design" search={Route.useSearch()} />;
}
