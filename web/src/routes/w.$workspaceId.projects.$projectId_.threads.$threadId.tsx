import { useCallback, useEffect, useRef, useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import { loadThread } from "@/api/threads";
import { DecisionBodySchema, type DecisionBody, type PostBody, type ThreadBody } from "@/api/generated";
import { PageHeader } from "@/components/page-header";
import { Trail } from "@/components/trail";
import { Composer } from "@/components/thread/composer";
import { DecisionPanel } from "@/components/thread/decision-panel";
import { Segmented } from "@/components/thread/segmented";
import { Phases } from "@/components/thread/timeline";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { useProjectEvents } from "@/live/context";

// The thread view is where the reviewer follows a thread live and decides on
// it: the posts grouped by phase (a sealed phase is labeled, but the human reads
// every post), a switch between full posts and summaries only, a form to post a
// directive, note or question (a reply links to the post it answers), and the
// current decision with Approve and Reject. The thread is loaded again on every
// live event of the project, so posts, phases and decisions appear without a
// reload.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/threads/$threadId")({
  component: ThreadView,
});

/** Loads the thread, and again after each call of `reload`; an answer that is no longer the newest is dropped. */
function useThread(workspaceId: string, projectId: string, threadId: string) {
  const [thread, setThread] = useState<ThreadBody | null>(null);
  const [failed, setFailed] = useState(false);
  const newest = useRef(0);

  const reload = useCallback(() => {
    newest.current += 1;
    const mine = newest.current;
    loadThread(workspaceId, projectId, threadId).then(
      (loaded) => {
        if (mine !== newest.current) return;
        setThread(loaded);
        setFailed(false);
      },
      () => {
        if (mine === newest.current) setFailed(true);
      },
    );
  }, [workspaceId, projectId, threadId]);

  useEffect(() => {
    setThread(null);
    reload();
    return () => {
      newest.current += 1;
    };
  }, [reload]);
  useProjectEvents(projectId, reload);
  return { thread, failed, reload };
}

type Show = "full" | "summaries";

const SHOW: { value: Show; label: string }[] = [
  { value: "full", label: "Full posts" },
  { value: "summaries", label: "Summaries only" },
];

function ThreadView() {
  const { workspaceId, projectId, threadId } = Route.useParams();
  const { thread, failed, reload } = useThread(workspaceId, projectId, threadId);
  const [show, setShow] = useState<Show>("full");
  const [replyTo, setReplyTo] = useState<PostBody | null>(null);
  const postCount = thread?.posts?.length ?? 0;

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader
        eyebrow={
          thread && <Trail workspaceId={workspaceId} projectId={projectId} featureId={thread.feature_id} section="threads" />
        }
        title={thread?.question ?? (failed ? "Thread" : "")}
        badge={
          thread?.run_tag ? (
            <Badge variant="outline" className="font-mono text-[11px] font-normal">
              {thread.run_tag}
            </Badge>
          ) : undefined
        }
        description={
          thread?.criteria &&
          thread.criteria.length > 0 && (
            <div className="space-y-2 pt-2">
              <div className="text-[11px] font-medium tracking-[0.08em] uppercase">Criteria</div>
              <ul className="flex flex-wrap gap-1.5">
                {thread.criteria.map((criterion) => (
                  <li
                    key={criterion}
                    className="rounded-md px-2 py-0.5 text-[12.5px] leading-5 text-foreground/80 ring-1 ring-border ring-inset"
                  >
                    {criterion}
                  </li>
                ))}
              </ul>
            </div>
          )
        }
      />
      {failed && (
        <Alert variant="destructive" className="mt-8">
          <AlertDescription>The thread could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {thread && (
        <div className="mt-10 grid grid-cols-[minmax(0,1fr)] gap-10 lg:grid-cols-[minmax(0,1fr)_360px] lg:items-start lg:gap-12">
          <aside className="min-w-0 lg:sticky lg:top-8 lg:col-start-2 lg:row-start-1">
            <DecisionPanel
              workspaceId={workspaceId}
              projectId={projectId}
              decision={parseDecision(thread)}
              posts={thread.posts ?? []}
              onChanged={reload}
            />
          </aside>
          <div className="min-w-0 space-y-8 lg:col-start-1 lg:row-start-1">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="flex items-baseline gap-2.5">
                <span className="text-[15px] font-semibold tracking-tight">Discussion</span>
                <span className="text-sm text-muted-foreground tabular-nums">
                  {postCount} {postCount === 1 ? "post" : "posts"}
                  {thread.phases && ` · ${thread.phases.length} ${thread.phases.length === 1 ? "phase" : "phases"}`}
                </span>
              </div>
              <Segmented name="show" label="Show" options={SHOW} value={show} onChange={setShow} />
            </div>
            <Phases
              workspaceId={workspaceId}
              projectId={projectId}
              thread={thread}
              summariesOnly={show === "summaries"}
              onReply={setReplyTo}
            />
            <Composer
              workspaceId={workspaceId}
              projectId={projectId}
              threadId={threadId}
              replyTo={replyTo}
              onCancelReply={() => setReplyTo(null)}
              onPosted={() => {
                setReplyTo(null);
                reload();
              }}
            />
          </div>
        </div>
      )}
      {!thread && !failed && <ThreadSkeleton />}
    </main>
  );
}

function ThreadSkeleton() {
  return (
    <div className="mt-10 grid gap-10 lg:grid-cols-[minmax(0,1fr)_360px] lg:gap-12" aria-hidden>
      <div className="space-y-4">
        {[0, 1, 2].map((row) => (
          <div key={row} className="h-20 animate-pulse rounded-xl bg-muted" />
        ))}
      </div>
      <div className="h-72 animate-pulse rounded-xl bg-muted" />
    </div>
  );
}

function parseDecision(thread: ThreadBody): DecisionBody | null {
  const parsed = DecisionBodySchema.safeParse(thread.decision);
  return parsed.success ? parsed.data : null;
}
