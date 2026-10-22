import { createFileRoute } from "@tanstack/react-router";
import type { DocBody, HomeResponse } from "@/api/generated";
import {
  ChangesetRow,
  DocRow,
  Empty,
  List,
  MODULES,
  Section,
  ThreadRow,
  ViewAll,
  WaitingList,
  waitingCount,
  waitingGroups,
} from "@/components/project/items";
import { PageHeader } from "@/components/page-header";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useHome, useProjectName } from "@/lib/use-home";

// The project home is the reviewer's overview of one project: its docs grouped
// by module, change sets with their status and threads, and a "waiting for
// you" section with everything that needs the human. Each part also has a page
// of its own (Inbox, Docs, Change sets, Threads). It loads the home from the
// API and loads it again on every live event of the project, so items appear
// and disappear without a reload.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId")({
  component: ProjectHome,
});

function ProjectHome() {
  const { workspaceId, projectId } = Route.useParams();
  const { home, failed } = useHome(workspaceId, projectId);
  const name = useProjectName(workspaceId, projectId);
  const count = home ? waitingCount(home) : 0;

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader
        eyebrow="Overview"
        title={name ?? "Project"}
        description={
          home &&
          (count
            ? `${count} ${count === 1 ? "thing waits" : "things wait"} for your call. Agents keep working on the rest.`
            : "You're all caught up. Agents keep working; anything that needs you shows up here.")
        }
      />
      {failed && (
        <Alert variant="destructive" className="mt-8">
          <AlertDescription>The project could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {home && (
        <div className="mt-10 space-y-12">
          <Waiting home={home} />
          <div className="grid gap-12 lg:grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)] lg:gap-10">
            <Docs docs={home.docs} />
            <div className="space-y-12">
              <ChangeSets home={home} />
              <Threads home={home} />
            </div>
          </div>
        </div>
      )}
      {!home && !failed && <HomeSkeleton />}
    </main>
  );
}

function HomeSkeleton() {
  return (
    <div className="mt-10 space-y-3" aria-hidden>
      {[0, 1, 2].map((row) => (
        <div key={row} className="h-14 animate-pulse rounded-xl bg-muted" />
      ))}
    </div>
  );
}

function Waiting({ home }: { home: HomeResponse }) {
  const params = Route.useParams();
  const count = waitingCount(home);
  return (
    <Section
      id="waiting"
      title="Waiting for you"
      count={count}
      more={<ViewAll to="/w/$workspaceId/projects/$projectId/inbox" params={params} />}
    >
      {!count && <Empty>Nothing waits for you.</Empty>}
      <div className="grid gap-6 md:grid-cols-2">
        {waitingGroups(home, params).map((group) => (
          <WaitingList key={group.kind} group={group} />
        ))}
      </div>
    </Section>
  );
}

function Docs({ docs }: { docs: DocBody[] }) {
  const params = Route.useParams();
  return (
    <Section
      id="docs"
      title="Docs"
      count={docs.length}
      more={<ViewAll to="/w/$workspaceId/projects/$projectId/docs" params={params} />}
    >
      {docs.length === 0 && <Empty>The project has no docs yet.</Empty>}
      <div className="space-y-6">
        {MODULES.map(({ module, title, blurb }) => {
          const inModule = docs.filter((doc) => doc.module === module);
          if (!inModule.length) return null;
          const id = `docs-${module}`;
          return (
            <section key={module} aria-labelledby={id} className="space-y-2">
              <div className="flex items-baseline gap-2">
                <h3 id={id} className="text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
                  {title}
                </h3>
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
      </div>
    </Section>
  );
}

function ChangeSets({ home }: { home: HomeResponse }) {
  const params = Route.useParams();
  return (
    <Section
      id="changesets"
      title="Change sets"
      count={home.changesets.length}
      more={<ViewAll to="/w/$workspaceId/projects/$projectId/changesets" params={params} />}
    >
      {home.changesets.length === 0 && <Empty>The project has no change sets yet.</Empty>}
      <List className="empty:hidden">
        {home.changesets.map((changeset) => (
          <ChangesetRow key={changeset.id} changeset={changeset} params={params} />
        ))}
      </List>
    </Section>
  );
}

function Threads({ home }: { home: HomeResponse }) {
  const params = Route.useParams();
  return (
    <Section
      id="threads"
      title="Threads"
      count={home.threads.length}
      more={<ViewAll to="/w/$workspaceId/projects/$projectId/threads" params={params} />}
    >
      {home.threads.length === 0 && <Empty>The project has no threads yet.</Empty>}
      <List className="empty:hidden">
        {home.threads.map((thread) => (
          <ThreadRow key={thread.id} thread={thread} params={params} />
        ))}
      </List>
    </Section>
  );
}
