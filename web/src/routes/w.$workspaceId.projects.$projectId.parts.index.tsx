import { createFileRoute } from "@tanstack/react-router";
import { PartsSection } from "@/components/project/parts-section";
import { Page, PageHead } from "@/components/shell/page";

// The project's Parts. Until S2 builds it, the parts section of the old project
// page: each part with its kind and description, and creating, renaming and
// deleting one.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/")({
  staticData: { tab: "parts" },
  component: Parts,
});

function Parts() {
  const { workspaceId, projectId } = Route.useParams();
  return (
    <Page>
      <PageHead title="Parts" />
      <div className="mt-6">
        <PartsSection workspaceId={workspaceId} projectId={projectId} />
      </div>
    </Page>
  );
}
