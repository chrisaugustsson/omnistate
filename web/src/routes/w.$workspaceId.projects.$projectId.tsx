import { useCallback, type ReactNode } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import {
  ChevronRight,
  CircleHelp,
  FileText,
  GitPullRequestArrow,
  MessageSquareText,
  MessagesSquare,
  PenTool,
  Scale,
  type LucideIcon,
} from "lucide-react";
import { loadProjects } from "@/api/home";
import type { DocBody, HomeResponse } from "@/api/generated";
import { AgentChip } from "@/components/agent-chip";
import { Eyebrow, PageHeader } from "@/components/page-header";
import { StatusBadge } from "@/components/status-badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { authorName, timeAgo } from "@/lib/format";
import { useReloading } from "@/lib/reloading";
import { useHome } from "@/lib/use-home";
import { cn } from "@/lib/utils";

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
  { module: "prd", title: "PRD", blurb: "What gets built and why" },
  { module: "architecture", title: "Architecture", blurb: "How the system fits together" },
  { module: "db", title: "DB", blurb: "The data model" },
  { module: "ui", title: "UI", blurb: "Design system and screens" },
];

function useProjectName(workspaceId: string, projectId: string) {
  const load = useCallback(() => loadProjects(workspaceId), [workspaceId]);
  const { value } = useReloading(load);
  return value?.find((project) => project.id === projectId)?.name;
}

function waitingCount(home: HomeResponse): number {
  const { decisions, changesets, comments, questions } = home.waiting;
  return decisions.length + changesets.length + comments.length + questions.length;
}

function ProjectHome() {
  const { workspaceId, projectId } = Route.useParams();
  const { home, failed } = useHome(workspaceId, projectId);
  const name = useProjectName(workspaceId, projectId);
  const count = home ? waitingCount(home) : 0;

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader
        eyebrow="Project"
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

function Section({
  id,
  title,
  count,
  children,
}: {
  id: string;
  title: string;
  count?: number;
  children: ReactNode;
}) {
  return (
    <section id={id} aria-labelledby={`${id}-title`} className="scroll-mt-6 space-y-4">
      <div className="flex items-baseline gap-2.5">
        <h2 id={`${id}-title`} className="text-[15px] font-semibold tracking-tight">
          {title}
        </h2>
        {count !== undefined && <span className="text-sm text-muted-foreground tabular-nums">{count}</span>}
      </div>
      {children}
    </section>
  );
}

function Empty({ children }: { children: ReactNode }) {
  return (
    <p className="rounded-xl border border-dashed px-4 py-6 text-center text-sm text-muted-foreground">{children}</p>
  );
}

/** A rounded list with hairline rules between its rows. */
function List({ children, className }: { children: ReactNode; className?: string }) {
  return <ul className={cn("divide-y overflow-hidden rounded-xl border bg-card", className)}>{children}</ul>;
}

const rowClass =
  "group flex items-center gap-3 px-4 py-3 text-sm transition-colors hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none";

/** A square icon tile in front of a row. */
function Glyph({ icon: Icon, tone }: { icon: LucideIcon; tone: string }) {
  return (
    <span className={cn("flex size-8 shrink-0 items-center justify-center rounded-lg", tone)}>
      <Icon className="size-4" />
    </span>
  );
}

type WaitingItem = { id: string; text: string; meta?: ReactNode; link?: ReactNode };

function Waiting({ home }: { home: HomeResponse }) {
  const { workspaceId, projectId } = Route.useParams();
  const { decisions, changesets, comments, questions } = home.waiting;
  const nothing = !decisions.length && !changesets.length && !comments.length && !questions.length;
  const params = { workspaceId, projectId };
  return (
    <Section id="waiting" title="Waiting for you" count={waitingCount(home)}>
      {nothing && <Empty>Nothing waits for you.</Empty>}
      <div className="grid gap-6 md:grid-cols-2">
        <WaitingList
          title="Decisions to approve"
          icon={Scale}
          tone="bg-signal-soft text-signal"
          items={decisions.map((d) => ({
            id: d.id,
            text: d.direction,
            meta: <AgentChip name={authorName(d.author)} />,
            link: (
              <Link
                to="/w/$workspaceId/projects/$projectId/threads/$threadId"
                params={{ ...params, threadId: d.thread_id }}
                aria-label="Open the decision"
                className="absolute inset-0"
              />
            ),
          }))}
        />
        <WaitingList
          title="Change sets in review"
          icon={GitPullRequestArrow}
          tone="bg-amber-50 text-amber-700 dark:bg-amber-400/10 dark:text-amber-300"
          items={changesets.map((c) => ({
            id: c.id,
            text: c.title,
            meta: <span>{timeAgo(c.created_at)}</span>,
            link: (
              <Link
                to="/w/$workspaceId/projects/$projectId/changesets/$changesetId"
                params={{ ...params, changesetId: c.id }}
                aria-label="Open the change set"
                className="absolute inset-0"
              />
            ),
          }))}
        />
        <WaitingList
          title="Comments marked addressed"
          icon={MessageSquareText}
          tone="bg-sky-50 text-sky-700 dark:bg-sky-400/10 dark:text-sky-300"
          items={comments.map((c) => ({
            id: c.id,
            text: c.text,
            meta: <span className="font-mono text-[11px]">{c.node}</span>,
            link: (
              <Link
                to="/w/$workspaceId/projects/$projectId/docs/$docId"
                params={{ ...params, docId: c.doc_id }}
                aria-label="Open the comment"
                className="absolute inset-0"
              />
            ),
          }))}
        />
        <WaitingList
          title="Questions from agents"
          icon={CircleHelp}
          tone="bg-violet-50 text-violet-700 dark:bg-violet-400/10 dark:text-violet-300"
          items={questions.map((q) => ({
            id: q.id,
            text: q.summary,
            meta: <AgentChip name={authorName(q.author)} />,
            link: (
              <Link
                to="/w/$workspaceId/projects/$projectId/threads/$threadId"
                params={{ ...params, threadId: q.thread_id }}
                aria-label="Answer the question"
                className="absolute inset-0"
              />
            ),
          }))}
        />
      </div>
    </Section>
  );
}

function WaitingList({
  title,
  icon,
  tone,
  items,
}: {
  title: string;
  icon: LucideIcon;
  tone: string;
  items: WaitingItem[];
}) {
  if (!items.length) return null;
  return (
    <div className="space-y-2">
      <Eyebrow>{title}</Eyebrow>
      <List>
        {items.map((item) => (
          <li key={item.id} className={cn(rowClass, "relative items-start")}>
            <Glyph icon={icon} tone={tone} />
            <div className="min-w-0 flex-1 space-y-1">
              <div className="leading-snug font-medium text-foreground">{item.text}</div>
              {item.meta && <div className="flex items-center gap-2 text-xs text-muted-foreground">{item.meta}</div>}
            </div>
            <ChevronRight className="mt-2 size-4 shrink-0 text-muted-foreground/50 transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
            {item.link}
          </li>
        ))}
      </List>
    </div>
  );
}

function Docs({ docs }: { docs: DocBody[] }) {
  const { workspaceId, projectId } = Route.useParams();
  return (
    <Section id="docs" title="Docs" count={docs.length}>
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
                {inModule.map((doc) => {
                  const design = doc.kind === "design";
                  return (
                    <li key={doc.id}>
                      <Link
                        to="/w/$workspaceId/projects/$projectId/docs/$docId"
                        params={{ workspaceId, projectId, docId: doc.id }}
                        aria-label={doc.title}
                        className={rowClass}
                      >
                        <Glyph
                          icon={design ? PenTool : FileText}
                          tone={design ? "bg-violet-50 text-violet-700 dark:bg-violet-400/10 dark:text-violet-300" : "bg-secondary text-muted-foreground ring-1 ring-border ring-inset"}
                        />
                        <span className="min-w-0 flex-1 truncate font-medium">{doc.title}</span>
                        {doc.claims.length > 0 && (
                          <span className="hidden items-center gap-1.5 text-xs text-signal sm:inline-flex">
                            <span className="relative flex size-1.5">
                              <span className="absolute inline-flex size-full animate-ping rounded-full bg-signal opacity-60" />
                              <span className="relative inline-flex size-1.5 rounded-full bg-signal" />
                            </span>
                            Agents editing
                          </span>
                        )}
                        <span className="hidden text-xs text-muted-foreground sm:inline">{design ? "Design" : "Doc"}</span>
                        <ChevronRight className="size-4 shrink-0 text-muted-foreground/50 transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
                      </Link>
                    </li>
                  );
                })}
              </List>
            </section>
          );
        })}
      </div>
    </Section>
  );
}

function ChangeSets({ home }: { home: HomeResponse }) {
  const { workspaceId, projectId } = Route.useParams();
  return (
    <Section id="changesets" title="Change sets" count={home.changesets.length}>
      {home.changesets.length === 0 && <Empty>The project has no change sets yet.</Empty>}
      <List className="empty:hidden">
        {home.changesets.map((changeset) => (
          <li key={changeset.id}>
            <Link
              to="/w/$workspaceId/projects/$projectId/changesets/$changesetId"
              params={{ workspaceId, projectId, changesetId: changeset.id }}
              className={rowClass}
            >
              <GitPullRequestArrow className="size-4 shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate font-medium">{changeset.title}</span>{" "}
              <StatusBadge status={changeset.status} />
            </Link>
          </li>
        ))}
      </List>
    </Section>
  );
}

function Threads({ home }: { home: HomeResponse }) {
  const { workspaceId, projectId } = Route.useParams();
  return (
    <Section id="threads" title="Threads" count={home.threads.length}>
      {home.threads.length === 0 && <Empty>The project has no threads yet.</Empty>}
      <List className="empty:hidden">
        {home.threads.map((thread) => {
          const phase = thread.phases.at(-1);
          return (
            <li key={thread.id}>
              <Link
                to="/w/$workspaceId/projects/$projectId/threads/$threadId"
                params={{ workspaceId, projectId, threadId: thread.id }}
                aria-label={thread.question}
                className={cn(rowClass, "items-start")}
              >
                <MessagesSquare className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 space-y-1">
                  <span className="block leading-snug font-medium">{thread.question}</span>
                  <span className="flex items-center gap-2 text-xs text-muted-foreground">
                    {phase && (
                      <span>
                        Phase {phase.number}
                        {phase.sealed ? " · sealed" : ""}
                      </span>
                    )}
                    {thread.run_tag && <span className="font-mono text-[11px]">{thread.run_tag}</span>}
                  </span>
                </span>
              </Link>
            </li>
          );
        })}
      </List>
    </Section>
  );
}
