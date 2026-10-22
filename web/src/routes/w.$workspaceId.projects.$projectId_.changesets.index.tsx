import { useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import { ChangesetRow, Empty, List } from "@/components/project/items";
import { FilterTabs, SectionPage } from "@/components/project/section-page";
import { statusWords } from "@/components/status-badge";

// The project's change sets, newest first, with a tab per status they have.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/changesets/")({
  component: ChangeSets,
});

/** The statuses in the order the tabs show them; others follow. */
const ORDER = ["in_review", "open", "approved", "merged", "rejected", "closed"];

function ChangeSets() {
  const { workspaceId, projectId } = Route.useParams();
  const [tab, setTab] = useState("all");
  const params = { workspaceId, projectId };

  return (
    <SectionPage
      workspaceId={workspaceId}
      projectId={projectId}
      title="Change sets"
      description={() => "Every change to a doc comes as a change set. You approve or reject it as a whole."}
    >
      {(home) => {
        const statuses = [...new Set(home.changesets.map((changeset) => changeset.status))].sort(
          (a, b) => (ORDER.indexOf(a) + 1 || 99) - (ORDER.indexOf(b) + 1 || 99),
        );
        const shown = home.changesets
          .filter((changeset) => tab === "all" || changeset.status === tab)
          .sort((a, b) => b.created_at.localeCompare(a.created_at));
        return (
          <>
            <FilterTabs
              label="Status"
              value={tab}
              onChange={setTab}
              tabs={[
                { value: "all", label: "All", count: home.changesets.length },
                ...statuses.map((status) => ({
                  value: status,
                  label: statusWords(status).replace(/^./, (first) => first.toUpperCase()),
                  count: home.changesets.filter((changeset) => changeset.status === status).length,
                })),
              ]}
            />
            {shown.length === 0 ? (
              <Empty>No change sets here yet.</Empty>
            ) : (
              <List>
                {shown.map((changeset) => (
                  <ChangesetRow key={changeset.id} changeset={changeset} params={params} detailed />
                ))}
              </List>
            )}
          </>
        );
      }}
    </SectionPage>
  );
}
