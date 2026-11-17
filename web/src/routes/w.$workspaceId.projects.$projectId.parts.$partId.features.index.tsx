import { createFileRoute } from "@tanstack/react-router";
import { FeaturesSection } from "@/components/project/features-section";
import { Page, PageHead } from "@/components/shell/page";

// A part's Features. Until S2 builds it, the features section of the old part
// page: each feature with what it does, and creating, renaming and deleting one.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/")({
  staticData: { tab: "features" },
  component: Features,
});

function Features() {
  const { workspaceId, projectId, partId } = Route.useParams();
  return (
    <Page>
      <PageHead title="Features" />
      <div className="mt-6">
        <FeaturesSection workspaceId={workspaceId} projectId={projectId} partId={partId} />
      </div>
    </Page>
  );
}
