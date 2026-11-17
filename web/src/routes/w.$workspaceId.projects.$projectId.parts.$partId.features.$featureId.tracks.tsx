import { createFileRoute } from "@tanstack/react-router";
import { Page } from "@/components/shell/page";

// A feature's Tracks tab. Slice S3 builds it.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/tracks")({
  component: Tracks,
});

function Tracks() {
  return <Page object><p className="text-ink-2">The tracks that change the feature, grouped by state.</p></Page>;
}
