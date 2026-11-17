import { createFileRoute } from "@tanstack/react-router";
import { ProjectDecisions } from "@/components/project/decisions";
import { Page, PageHead } from "@/components/shell/page";

// The project's Decisions. Until S2 builds the decision log, the project's
// decisions section as it was on the old project page.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/decisions")({
  staticData: { tab: "decisions" },
  component: Decisions,
});

function Decisions() {
  const { workspaceId, projectId } = Route.useParams();
  return (
    <Page>
      <PageHead title="Decisions" />
      <div className="mt-6">
        <ProjectDecisions workspaceId={workspaceId} projectId={projectId} />
      </div>
    </Page>
  );
}
