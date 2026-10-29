import { useEffect } from "react";
import { Link } from "@tanstack/react-router";
import { Check, Lock } from "lucide-react";
import type { HomeDoc, HomeResponse, HomeThread } from "@/api/generated";
import { Empty, List, PILLARS } from "@/components/project/items";
import { PageHeader } from "@/components/page-header";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { timeAgo } from "@/lib/format";
import { useHome, useProjectName } from "@/lib/use-home";
import { cn } from "@/lib/utils";

// The scope page: the app level of a project or one of its features, on one
// layout. A header with the name, the description and the last activity, a
// row of chips that jump to the five pillars, a section per pillar with a row
// for each doc, and the scope's threads, newest post first. It loads the
// project's home and loads it again on every live event of the project, so docs
// and posts appear as agents write them.

type Params = { workspaceId: string; projectId: string };

const KIND_LABELS: Record<string, string> = { html: "HTML doc", design: "Design", "design-system": "Design system" };

const rowClass =
  "group flex items-center gap-3 px-4 py-3 text-sm transition-colors hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none";

/** The docs of a pillar, by title, with the design system first. */
function inOrder(docs: HomeDoc[]): HomeDoc[] {
  return [...docs].sort(
    (a, b) =>
      Number(b.kind === "design-system") - Number(a.kind === "design-system") ||
      a.title.toLowerCase().localeCompare(b.title.toLowerCase()),
  );
}

/** When the thread was last written to: its newest post, or its start while it has none. */
export function threadTime(thread: HomeThread): string {
  return thread.last_post_at ?? thread.created_at;
}

type Flag = "Concluded" | "Sealed" | "Open";

/** The thread's flags: the one that matters most, or with `each` both sealed and concluded when they hold. */
function flagsOf(thread: HomeThread, each: boolean): Flag[] {
  const flags: Flag[] = [];
  if (thread.conclusion) flags.push("Concluded");
  if (thread.sealed && (each || flags.length === 0)) flags.unshift("Sealed");
  return flags.length > 0 ? flags : ["Open"];
}

function DocRow({ doc, params }: { doc: HomeDoc; params: Params }) {
  const waiting = doc.waiting ?? 0;
  return (
    <li>
      <Link to="/w/$workspaceId/projects/$projectId/docs/$docId" params={{ ...params, docId: doc.id }} className={rowClass}>
        <span className="min-w-0 flex-1 truncate font-medium">{doc.title}</span>
        <span className="hidden w-28 shrink-0 text-xs text-muted-foreground sm:inline">
          {KIND_LABELS[doc.kind] ?? "HTML doc"}
        </span>
        <span
          title={doc.updated_at ? new Date(doc.updated_at).toLocaleString() : undefined}
          className="hidden w-52 shrink-0 truncate text-xs text-muted-foreground md:inline"
        >
          {doc.updated_at ? timeAgo(doc.updated_at) : ""}
          {doc.last_writer_role ? ` · ${doc.last_writer_role}` : ""}
        </span>
        <span className="flex w-8 shrink-0 justify-end">
          {waiting > 0 && (
            <span
              title="Waiting on you"
              className="rounded-full bg-signal px-1.5 text-center text-[11px] leading-5 text-signal-foreground tabular-nums"
            >
              {waiting}
            </span>
          )}
        </span>
      </Link>
    </li>
  );
}

/**
 * A thread's row; `scopeName` adds the scope that the thread belongs to, for lists that mix scopes, and
 * `eachFlag` shows both Sealed and Concluded when they hold, where otherwise only Concluded shows.
 */
export function ThreadRow({
  thread,
  params,
  scopeName,
  eachFlag = false,
}: {
  thread: HomeThread;
  params: Params;
  scopeName?: string;
  eachFlag?: boolean;
}) {
  const flags = flagsOf(thread, eachFlag);
  const posts = thread.post_count ?? 0;
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/threads/$threadId"
        params={{ ...params, threadId: thread.id }}
        className={rowClass}
      >
        <span className="min-w-0 flex-1 truncate font-medium">{thread.question}</span>
        {scopeName && (
          <span className="hidden max-w-40 shrink-0 truncate text-xs text-muted-foreground sm:inline">{scopeName}</span>
        )}
        <span className="hidden w-20 shrink-0 text-xs text-muted-foreground sm:inline">
          {posts} {posts === 1 ? "post" : "posts"}
        </span>
        <span
          title={new Date(threadTime(thread)).toLocaleString()}
          className="hidden w-24 shrink-0 text-xs text-muted-foreground md:inline"
        >
          {timeAgo(threadTime(thread))}
        </span>
        <span
          className={cn(
            "flex shrink-0 items-center justify-end gap-3 text-xs text-muted-foreground",
            eachFlag ? "w-44" : "w-24",
          )}
        >
          {flags.map((flag) => (
            <span
              key={flag}
              className={cn(
                "flex items-center gap-1",
                flag === "Concluded" && "text-emerald-700 dark:text-emerald-400",
              )}
            >
              {flag === "Concluded" && <Check aria-hidden className="size-3.5" />}
              {flag === "Sealed" && <Lock aria-hidden className="size-3.5" />}
              {flag}
            </span>
          ))}
        </span>
      </Link>
    </li>
  );
}

function Skeleton() {
  return (
    <div className="mt-10 space-y-3" aria-hidden>
      {[0, 1, 2].map((row) => (
        <div key={row} className="h-14 animate-pulse rounded-xl bg-muted" />
      ))}
    </div>
  );
}

/** The scope's docs and threads out of `home`: a feature's, or the app level's when `featureId` is null. */
function inScope(home: HomeResponse, featureId: string | null) {
  const own = (item: { feature_id?: string | null }) => (item.feature_id ?? null) === featureId;
  return {
    docs: home.docs.filter(own),
    threads: home.threads
      .filter(own)
      .sort((a, b) => threadTime(b).localeCompare(threadTime(a))),
  };
}

/** Puts the page at the section that the address names, once the sections are there. */
function useScrollToHash(ready: boolean) {
  useEffect(() => {
    if (!ready) return;
    const id = decodeURIComponent(window.location.hash.slice(1));
    if (id) document.getElementById(id)?.scrollIntoView();
  }, [ready]);
}

export function ScopePage({ workspaceId, projectId, featureId }: Params & { featureId: string | null }) {
  const params = { workspaceId, projectId };
  const { home, failed } = useHome(workspaceId, projectId);
  const projectName = useProjectName(workspaceId, projectId);
  const summary = featureId ? home?.features?.find((candidate) => candidate.feature.id === featureId) : undefined;
  const missing = home !== null && featureId !== null && !summary;
  const ready = home !== null && !missing;
  useScrollToHash(ready);

  const title = featureId ? (summary?.feature.name ?? "") : (projectName ?? "");
  const description = featureId ? summary?.feature.description : undefined;
  const lastActivity = featureId ? summary?.last_activity : home?.app?.last_activity;
  const { docs, threads } = home && ready ? inScope(home, featureId) : { docs: [], threads: [] };

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      {failed && (
        <Alert variant="destructive">
          <AlertDescription>The page could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {missing && (
        <div className="space-y-3">
          <h1 className="font-display text-[2.125rem] leading-[1.1] tracking-[-0.01em]">Feature not found</h1>
          <p className="text-[15px] text-muted-foreground">
            The project has no such feature, or it was deleted.{" "}
            <Link
              to="/w/$workspaceId/projects/$projectId/app"
              params={params}
              className="font-medium text-foreground underline underline-offset-4"
            >
              Go to App
            </Link>
          </p>
        </div>
      )}
      {!home && !failed && <Skeleton />}
      {home && ready && (
        <>
          <PageHeader
            title={title}
            description={
              <>
                {description && <p className="whitespace-pre-line">{description}</p>}
                <p className="mt-2 text-xs text-muted-foreground" title={lastActivity ? new Date(lastActivity).toLocaleString() : undefined}>
                  {lastActivity ? `Last change ${timeAgo(lastActivity)}` : "No activity yet"}
                </p>
              </>
            }
          />
          <nav aria-label="Pillars" className="mt-6 flex flex-wrap gap-1.5">
            {PILLARS.map(({ pillar, title: label }) => (
              <a
                key={pillar}
                href={`#${pillar}`}
                className="inline-flex h-7 items-center gap-1.5 rounded-lg border px-2.5 text-[12.5px] text-foreground/80 transition-colors hover:bg-accent"
              >
                {label}
                <span className="text-muted-foreground tabular-nums">{docs.filter((doc) => doc.pillar === pillar).length}</span>
              </a>
            ))}
          </nav>
          <div className="mt-10 space-y-12">
            {PILLARS.map(({ pillar, title: label }) => {
              const inPillar = inOrder(docs.filter((doc) => doc.pillar === pillar));
              const id = `${pillar}-title`;
              return (
                <section key={pillar} id={pillar} aria-labelledby={id} className="scroll-mt-6 space-y-3">
                  <div className="flex items-baseline gap-2.5">
                    <h2 id={id} className="text-[15px] font-semibold tracking-tight">
                      {label}
                    </h2>
                    <span className="text-sm text-muted-foreground tabular-nums">{inPillar.length}</span>
                  </div>
                  {inPillar.length === 0 ? (
                    <Empty>No docs yet</Empty>
                  ) : (
                    <List>
                      {inPillar.map((doc) => (
                        <DocRow key={doc.id} doc={doc} params={params} />
                      ))}
                    </List>
                  )}
                </section>
              );
            })}
            <section id="threads" aria-labelledby="threads-title" className="scroll-mt-6 space-y-3">
              <div className="flex items-baseline gap-2.5">
                <h2 id="threads-title" className="text-[15px] font-semibold tracking-tight">
                  Threads
                </h2>
                <span className="text-sm text-muted-foreground tabular-nums">{threads.length}</span>
              </div>
              {threads.length === 0 ? (
                <Empty>No threads yet</Empty>
              ) : (
                <List>
                  {threads.map((thread) => (
                    <ThreadRow key={thread.id} thread={thread} params={params} />
                  ))}
                </List>
              )}
            </section>
          </div>
        </>
      )}
    </main>
  );
}
