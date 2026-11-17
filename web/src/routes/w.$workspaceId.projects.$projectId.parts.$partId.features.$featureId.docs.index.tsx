import { createFileRoute } from "@tanstack/react-router";
import { Page } from "@/components/shell/page";

// A feature's Docs: the docs on the feature beside the open doc. Slice S3 builds it.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/docs/")({
  component: Docs,
});

function Docs() {
  return <Page object><p className="text-ink-2">The feature&rsquo;s docs, beside the open doc.</p></Page>;
}
