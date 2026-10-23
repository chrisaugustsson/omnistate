import { useState } from "react";
import type { ThreadBody } from "@/api/generated";
import { ChangesetRow, DocRow, Empty, List, MODULES, ThreadRow, isDesign } from "@/components/project/items";
import { FilterTabs, SectionPage } from "@/components/project/section-page";
import { statusWords } from "@/components/status-badge";

// The Docs, Change sets and Threads pages of a project and of each of its
// features: the same page, over the project's own items (`featureId` unset) or
// over one feature's.

type PageProps = { workspaceId: string; projectId: string; featureId?: string };

type DocsTab = "all" | "html" | "design";

/** The docs: the PRD, architecture, DB and UI docs, grouped by module, with a tab for written docs and one for designs. */
export function DocsPage({ workspaceId, projectId, featureId }: PageProps) {
  const [tab, setTab] = useState<DocsTab>("all");
  const params = { workspaceId, projectId };

  return (
    <SectionPage
      workspaceId={workspaceId}
      projectId={projectId}
      feature={featureId ?? null}
      title="Docs"
      description={() => "The plans agents write: the PRD, the architecture, the database and the UI."}
    >
      {(home) => {
        const docs = home.docs.filter((doc) => tab === "all" || (tab === "design") === isDesign(doc));
        return (
          <>
            <FilterTabs
              label="Kind of doc"
              value={tab}
              onChange={setTab}
              tabs={[
                { value: "all", label: "All", count: home.docs.length },
                { value: "html", label: "Docs", count: home.docs.filter((doc) => !isDesign(doc)).length },
                { value: "design", label: "Designs", count: home.docs.filter(isDesign).length },
              ]}
            />
            {docs.length === 0 && <Empty>No docs here yet. Agents add them through change sets.</Empty>}
            {MODULES.map(({ module, title, blurb }) => {
              const inModule = docs.filter((doc) => doc.module === module);
              if (!inModule.length) return null;
              const id = `docs-${module}`;
              return (
                <section key={module} aria-labelledby={id} className="space-y-2">
                  <div className="flex items-baseline gap-2">
                    <h2 id={id} className="text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
                      {title}
                    </h2>
                    <span className="text-xs text-muted-foreground/70">{blurb}</span>
                  </div>
                  <List>
                    {inModule.map((doc) => (
                      <DocRow key={doc.id} doc={doc} params={params} />
                    ))}
                  </List>
                </section>
              );
            })}
          </>
        );
      }}
    </SectionPage>
  );
}

/** The statuses in the order the tabs show them; others follow. */
const ORDER = ["in_review", "open", "approved", "merged", "rejected", "closed"];

/** The change sets, newest first, with a tab per status they have. */
export function ChangesetsPage({ workspaceId, projectId, featureId }: PageProps) {
  const [tab, setTab] = useState("all");
  const params = { workspaceId, projectId };

  return (
    <SectionPage
      workspaceId={workspaceId}
      projectId={projectId}
      feature={featureId ?? null}
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

type ThreadsTab = "all" | "open" | "decided";

function decided(thread: ThreadBody): boolean {
  return thread.decision != null;
}

/** The threads: the forum where agents work out each question, with tabs for threads still being discussed and threads with a decision. */
export function ThreadsPage({ workspaceId, projectId, featureId }: PageProps) {
  const [tab, setTab] = useState<ThreadsTab>("all");
  const params = { workspaceId, projectId };

  return (
    <SectionPage
      workspaceId={workspaceId}
      projectId={projectId}
      feature={featureId ?? null}
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
