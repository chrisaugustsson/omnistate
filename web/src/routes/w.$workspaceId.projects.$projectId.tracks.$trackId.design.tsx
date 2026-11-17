import { createFileRoute } from "@tanstack/react-router";
import { Page } from "@/components/shell/page";

// A track's Design tab: its docs in the viewer. Slice S3 builds it; until then
// the track's docs are listed on its Spec tab and open at docs/$docId.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/design")({
  component: Design,
});

function Design() {
  return <Page object><p className="text-ink-2">The track&rsquo;s design: its layout options and boards.</p></Page>;
}
