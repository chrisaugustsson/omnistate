import { createFileRoute } from "@tanstack/react-router";
import { PartDecisions } from "@/components/project/decisions";
import { Page, PageHead } from "@/components/shell/page";

// A part's Decisions. Until S2 builds the decision log, the part's decisions
// section as it was on the old part page.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/decisions")({
  staticData: { tab: "decisions" },
  component: Decisions,
});

function Decisions() {
  const { workspaceId, projectId, partId } = Route.useParams();
  return (
    <Page>
      <PageHead title="Decisions" />
      <div className="mt-6">
        <PartDecisions workspaceId={workspaceId} projectId={projectId} partId={partId} />
      </div>
    </Page>
  );
}
