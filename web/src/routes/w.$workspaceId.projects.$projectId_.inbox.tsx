import { useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import { PartyPopper } from "lucide-react";
import { WaitingList, featureNames, waitingCount, waitingGroups, type WaitingGroup } from "@/components/project/items";
import { FilterTabs, SectionPage } from "@/components/project/section-page";

// The project's inbox: everything that waits for the reviewer (decisions to
// approve, change sets in review, comments marked addressed and agents'
// questions), by kind, with a tab per kind. The items of a feature carry the
// feature's name.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/inbox")({
  component: Inbox,
});

type Tab = "all" | WaitingGroup["kind"];

const SHORT: Record<WaitingGroup["kind"], string> = {
  decisions: "Decisions",
  changesets: "Change sets",
  comments: "Comments",
  questions: "Questions",
};

function Inbox() {
  const { workspaceId, projectId } = Route.useParams();
  const [tab, setTab] = useState<Tab>("all");

  return (
    <SectionPage
      workspaceId={workspaceId}
      projectId={projectId}
      title="Inbox"
      description={(home) => {
        const count = waitingCount(home);
        return count
          ? `${count} ${count === 1 ? "thing waits" : "things wait"} for your call.`
          : "Nothing waits for you. Anything that needs you shows up here.";
      }}
    >
      {(home) => {
        const groups = waitingGroups(home, { workspaceId, projectId }, featureNames(home));
        const shown = groups.filter((group) => tab === "all" || group.kind === tab);
        const nothing = shown.every((group) => !group.items.length);
        return (
          <>
            <FilterTabs
              label="What waits"
              value={tab}
              onChange={setTab}
              tabs={[
                { value: "all", label: "All", count: waitingCount(home) },
                ...groups.map((group) => ({ value: group.kind, label: SHORT[group.kind], count: group.items.length })),
              ]}
            />
            {nothing ? (
              <div className="rounded-xl border border-dashed px-6 py-14 text-center">
                <PartyPopper className="mx-auto size-6 text-muted-foreground" />
                <p className="mt-3 text-sm font-medium">You're all caught up</p>
                <p className="mt-1 text-sm text-muted-foreground">Agents keep working; new items appear here live.</p>
              </div>
            ) : (
              <div className="space-y-8">
                {shown.map((group) => (
                  <WaitingList key={group.kind} group={group} showTitle={tab === "all"} />
                ))}
              </div>
            )}
          </>
        );
      }}
    </SectionPage>
  );
}
