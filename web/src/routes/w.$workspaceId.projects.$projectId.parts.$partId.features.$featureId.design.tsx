import { createFileRoute } from "@tanstack/react-router";
import { Page } from "@/components/shell/page";

// A feature's Design tab. Slice S3 builds it.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/design")({
  component: Design,
});

function Design() {
  return <Page object><p className="text-ink-2">The feature&rsquo;s design docs.</p></Page>;
}
