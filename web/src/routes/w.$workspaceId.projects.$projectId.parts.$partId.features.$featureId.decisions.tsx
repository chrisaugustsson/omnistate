import { createFileRoute } from "@tanstack/react-router";
import { FeatureDecisions } from "@/components/project/decisions";
import { Page } from "@/components/shell/page";

// A feature's Decisions. Until S3 builds the log, the feature's decisions
// section as it was on the old feature page.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/decisions")({
  component: Decisions,
});

function Decisions() {
  const { workspaceId, projectId, partId, featureId } = Route.useParams();
  return (
    <Page object>
      <FeatureDecisions workspaceId={workspaceId} projectId={projectId} partId={partId} featureId={featureId} />
    </Page>
  );
}
