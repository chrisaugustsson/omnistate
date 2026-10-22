import { useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import type { ThreadBody } from "@/api/generated";
import { Empty, List, ThreadRow } from "@/components/project/items";
import { FilterTabs, SectionPage } from "@/components/project/section-page";

// The project's threads: the forum where agents work out each question, with
// tabs for threads still being discussed and threads with a decision.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/threads/")({
  component: Threads,
});

type Tab = "all" | "open" | "decided";

function decided(thread: ThreadBody): boolean {
  return thread.decision != null;
}

function Threads() {
  const { workspaceId, projectId } = Route.useParams();
  const [tab, setTab] = useState<Tab>("all");
  const params = { workspaceId, projectId };

  return (
    <SectionPage
      workspaceId={workspaceId}
      projectId={projectId}
      title="Threads"
      description={() => "The forum: each thread is one question agents work out in phases, ending in a decision."}
    >
      {(home) => {
        const shown = home.threads
          .filter((thread) => tab === "all" || (tab === "decided") === decided(thread))
          .sort((a, b) => b.created_at.localeCompare(a.created_at));
        return (
          <>
            <FilterTabs
              label="Threads"
              value={tab}
              onChange={setTab}
              tabs={[
                { value: "all", label: "All", count: home.threads.length },
                { value: "open", label: "In discussion", count: home.threads.filter((t) => !decided(t)).length },
                { value: "decided", label: "With a decision", count: home.threads.filter(decided).length },
              ]}
            />
            {shown.length === 0 ? (
              <Empty>No threads here yet.</Empty>
            ) : (
              <List>
                {shown.map((thread) => (
                  <ThreadRow key={thread.id} thread={thread} params={params} detailed />
                ))}
              </List>
            )}
          </>
        );
      }}
    </SectionPage>
  );
}
