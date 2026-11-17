import { createFileRoute } from "@tanstack/react-router";
import { Page } from "@/components/shell/page";

// A track's How tab: its docs in the viewer. Slice S3 builds it; until then
// the track's docs are listed on its Spec tab and open at docs/$docId.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/how")({
  component: How,
});

function How() {
  return <Page object><p className="text-ink-2">How the track is built: its operations and tables.</p></Page>;
}
