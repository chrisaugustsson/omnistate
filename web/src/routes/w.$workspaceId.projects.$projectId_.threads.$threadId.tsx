import { useCallback, useEffect } from "react";
import { createFileRoute, Link, useLocation } from "@tanstack/react-router";
import { ChevronRight, Lock } from "lucide-react";
import { loadThread } from "@/api/threads";
import type { ThreadBody } from "@/api/generated";
import { PageHeader } from "@/components/page-header";
import { Composer } from "@/components/thread/composer";
import { Posts } from "@/components/thread/timeline";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { useHome } from "@/lib/use-home";
import { useReloading } from "@/lib/reloading";
import { useVisit } from "@/lib/visits";
import { cn } from "@/lib/utils";
import { useProjectEvents } from "@/live/context";

// The thread view is where the reviewer follows a thread live and posts in it:
// the question, the posts oldest first with new ones added at the bottom, a
// form to post (a summary and a Markdown body) and the conclusion once an agent
// has written it. The thread is loaded again on every live event of the
// project, so posts appear without a reload. `#post-<id>` scrolls to a post and
// highlights it.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/threads/$threadId")({
  component: ThreadView,
});

const crumbClass =
  "block max-w-48 truncate rounded transition-colors hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none";

/** Where the thread lives: its scope, the Threads page of that scope and the question. */
function ThreadTrail({ workspaceId, projectId, thread }: { workspaceId: string; projectId: string; thread: ThreadBody }) {
  const { home } = useHome(workspaceId, projectId);
  const featureId = thread.feature_id ?? null;
  const params = { workspaceId, projectId };
  const scopeName = featureId
    ? (home?.features?.find((summary) => summary.feature.id === featureId)?.feature.name ?? "Feature")
    : "App";
  return (
    <nav aria-label="Breadcrumb">
      <ol className="flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5 text-xs font-medium text-muted-foreground">
        <li className="min-w-0">
          {featureId ? (
            <Link
              to="/w/$workspaceId/projects/$projectId/features/$featureId"
              params={{ ...params, featureId }}
              className={crumbClass}
            >
              {scopeName}
            </Link>
          ) : (
            <Link to="/w/$workspaceId/projects/$projectId/app" params={params} className={crumbClass}>
              {scopeName}
            </Link>
          )}
        </li>
        <ChevronRight aria-hidden className="size-3 shrink-0 text-muted-foreground/50" />
        <li className="min-w-0">
          <Link
            to="/w/$workspaceId/projects/$projectId/threads"
            params={params}
            search={{ scope: featureId ?? "app" }}
            className={crumbClass}
          >
            Threads
          </Link>
        </li>
        <ChevronRight aria-hidden className="size-3 shrink-0 text-muted-foreground/50" />
        <li className="min-w-0">
          <span aria-current="page" className={cn(crumbClass, "text-foreground")}>
            {thread.question}
          </span>
        </li>
      </ol>
    </nav>
  );
}

function ThreadView() {
  const { workspaceId, projectId, threadId } = Route.useParams();
  const hash = useLocation({ select: (location) => location.hash });
  const loaded = useReloading(useCallback(() => loadThread(workspaceId, projectId, threadId), [workspaceId, projectId, threadId]));
  const { value: thread, failed, reload } = loaded;
  const known = thread?.id === threadId ? thread : null;
  useVisit(projectId, known ? (known.feature_id ?? "app") : undefined, undefined, known ? threadId : undefined);
  useProjectEvents(projectId, reload);

  const wanted = hash.startsWith("post-") ? hash.slice("post-".length) : null;
  const shown = thread?.posts?.some((post) => post.id === wanted) ? wanted : null;
  useEffect(() => {
    if (shown) document.getElementById(`post-${shown}`)?.scrollIntoView({ block: "center" });
  }, [shown]);

  return (
    <main className="mx-auto max-w-4xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader
        eyebrow={thread && <ThreadTrail workspaceId={workspaceId} projectId={projectId} thread={thread} />}
        title={thread?.question ?? (failed ? "Thread" : "")}
        badge={
          thread && (
            <>
              {thread.sealed && (
                <Badge variant="violet" className="px-1.5 py-0 text-[11px] leading-[18px]">
                  <Lock />
                  Sealed
                </Badge>
              )}
              {thread.run_tag && (
                <Badge variant="outline" className="font-mono text-[11px] font-normal">
                  {thread.run_tag}
                </Badge>
              )}
            </>
          )
        }
      />
      {failed && !thread && (
        <Alert variant="destructive" className="mt-8">
          <AlertDescription>The thread could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {thread && (
        <div className="mt-10 space-y-8">
          {thread.conclusion && (
            <section aria-labelledby="conclusion-title" className="space-y-2 rounded-xl border bg-card px-4 py-3">
              <h2 id="conclusion-title" className="text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
                Conclusion
              </h2>
              <p className="text-[14px] leading-relaxed whitespace-pre-wrap">{thread.conclusion}</p>
            </section>
          )}
          <div className="flex items-baseline gap-2.5">
            <h2 className="text-[15px] font-semibold tracking-tight">Discussion</h2>
            <span className="text-sm text-muted-foreground tabular-nums">
              {thread.posts?.length ?? 0} {thread.posts?.length === 1 ? "post" : "posts"}
            </span>
          </div>
          <Posts workspaceId={workspaceId} projectId={projectId} posts={thread.posts ?? []} highlighted={shown} />
          <Composer workspaceId={workspaceId} projectId={projectId} threadId={threadId} onPosted={reload} />
        </div>
      )}
      {!thread && !failed && <ThreadSkeleton />}
    </main>
  );
}

function ThreadSkeleton() {
  return (
    <div className="mt-10 space-y-4" aria-hidden>
      {[0, 1, 2].map((row) => (
        <div key={row} className="h-20 animate-pulse rounded-xl bg-muted" />
      ))}
    </div>
  );
}
