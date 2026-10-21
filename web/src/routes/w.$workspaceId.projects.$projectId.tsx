import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { createFileRoute } from "@tanstack/react-router";
import { loadHome } from "@/api/home";
import type { DocBody, HomeResponse } from "@/api/generated";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useProjectEvents } from "@/live/context";

// The project home is the reviewer's page of one project: its docs grouped by
// module, change sets with their status and threads, and a "waiting for you"
// section with everything that needs the human. It loads the home from the API
// and loads it again on every live event of the project, so items appear and
// disappear without a reload.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId")({
  component: ProjectHome,
});

/** The modules of a project's docs, in the order the page shows them. */
const MODULES = [
  { module: "prd", title: "PRD" },
  { module: "architecture", title: "Architecture" },
  { module: "db", title: "DB" },
  { module: "ui", title: "UI" },
];

/** A status as words: `in_review` is "in review". */
function statusWords(status: string): string {
  return status.replace(/_/g, " ");
}

/** Loads the home, and again after each call of `reload`; an answer that is no longer the newest is dropped. */
function useHome(workspaceId: string, projectId: string) {
  const [home, setHome] = useState<HomeResponse | null>(null);
  const [failed, setFailed] = useState(false);
  const newest = useRef(0);

  const reload = useCallback(() => {
    newest.current += 1;
    const mine = newest.current;
    loadHome(workspaceId, projectId).then(
      (loaded) => {
        if (mine !== newest.current) return;
        setHome(loaded);
        setFailed(false);
      },
      () => {
        if (mine === newest.current) setFailed(true);
      },
    );
  }, [workspaceId, projectId]);

  useEffect(() => {
    setHome(null);
    reload();
    return () => {
      newest.current += 1;
    };
  }, [reload]);
  useProjectEvents(projectId, reload);
  return { home, failed };
}

function ProjectHome() {
  const { workspaceId, projectId } = Route.useParams();
  const { home, failed } = useHome(workspaceId, projectId);

  return (
    <main className="mx-auto max-w-5xl space-y-8 p-6">
      {failed && (
        <Alert variant="destructive">
          <AlertDescription>The project could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {home && (
        <>
          <Waiting home={home} />
          <Docs docs={home.docs} />
          <ChangeSets home={home} />
          <Threads home={home} />
        </>
      )}
    </main>
  );
}

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="space-y-3">
      <h2 id={id} className="text-lg font-semibold">
        {title}
      </h2>
      {children}
    </section>
  );
}

function Empty({ children }: { children: ReactNode }) {
  return <p className="text-sm text-muted-foreground">{children}</p>;
}

function Waiting({ home }: { home: HomeResponse }) {
  const { decisions, changesets, comments, questions } = home.waiting;
  const nothing = !decisions.length && !changesets.length && !comments.length && !questions.length;
  return (
    <Section id="waiting" title="Waiting for you">
      {nothing && <Empty>Nothing waits for you.</Empty>}
      <WaitingList title="Decisions to approve" items={decisions.map((d) => ({ id: d.id, text: d.direction }))} />
      <WaitingList title="Change sets in review" items={changesets.map((c) => ({ id: c.id, text: c.title }))} />
      <WaitingList title="Comments marked addressed" items={comments.map((c) => ({ id: c.id, text: c.text }))} />
      <WaitingList title="Questions from agents" items={questions.map((q) => ({ id: q.id, text: q.summary }))} />
    </Section>
  );
}

function WaitingList({ title, items }: { title: string; items: { id: string; text: string }[] }) {
  if (!items.length) return null;
  return (
    <div>
      <h3 className="text-sm font-medium text-muted-foreground">{title}</h3>
      <ul className="mt-1 divide-y rounded-md border">
        {items.map((item) => (
          <li key={item.id} className="px-4 py-2 text-sm">
            {item.text}
          </li>
        ))}
      </ul>
    </div>
  );
}

function Docs({ docs }: { docs: DocBody[] }) {
  return (
    <Section id="docs" title="Docs">
      {docs.length === 0 && <Empty>The project has no docs yet.</Empty>}
      {MODULES.map(({ module, title }) => {
        const inModule = docs.filter((doc) => doc.module === module);
        if (!inModule.length) return null;
        const id = `docs-${module}`;
        return (
          <section key={module} aria-labelledby={id}>
            <h3 id={id} className="text-sm font-medium text-muted-foreground">
              {title}
            </h3>
            <ul className="mt-1 divide-y rounded-md border">
              {inModule.map((doc) => (
                <li key={doc.id} className="px-4 py-2 text-sm">
                  {doc.title}
                </li>
              ))}
            </ul>
          </section>
        );
      })}
    </Section>
  );
}

function ChangeSets({ home }: { home: HomeResponse }) {
  return (
    <Section id="changesets" title="Change sets">
      {home.changesets.length === 0 && <Empty>The project has no change sets yet.</Empty>}
      <ul className="divide-y rounded-md border empty:hidden">
        {home.changesets.map((changeset) => (
          <li key={changeset.id} className="flex items-center justify-between gap-3 px-4 py-2 text-sm">
            <span>{changeset.title}</span>{" "}
            <span className="rounded-full border px-2 py-0.5 text-xs text-muted-foreground">
              {statusWords(changeset.status)}
            </span>
          </li>
        ))}
      </ul>
    </Section>
  );
}

function Threads({ home }: { home: HomeResponse }) {
  return (
    <Section id="threads" title="Threads">
      {home.threads.length === 0 && <Empty>The project has no threads yet.</Empty>}
      <ul className="divide-y rounded-md border empty:hidden">
        {home.threads.map((thread) => (
          <li key={thread.id} className="px-4 py-2 text-sm">
            {thread.question}
          </li>
        ))}
      </ul>
    </Section>
  );
}
