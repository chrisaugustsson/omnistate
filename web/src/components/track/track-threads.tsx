import { useCallback, useEffect } from "react";
import { Link, useLocation } from "@tanstack/react-router";
import { CheckCircle2, Lock, MessageSquare } from "lucide-react";
import type { HomeThread } from "@/api/generated";
import { loadThread } from "@/api/threads";
import { Composer } from "@/components/thread/composer";
import { Posts } from "@/components/thread/timeline";
import { timeAgo } from "@/lib/format";
import { useReloading } from "@/lib/reloading";
import { cn } from "@/lib/utils";
import { useVisit } from "@/lib/visits";
import { useProjectEvents } from "@/live/context";
import { useTrack } from "./track-data";
import { Band, RowLead, rowClass } from "@/components/project/rows";
import { EmptyLine, shortDate } from "./ui";

// A track's Threads: its threads in two groups, open and concluded, newest
// post first, each opening in place under the list (…/threads/<thread>), where
// the reader follows it live and posts in it.

/** When the thread was last written to: its newest post, or its start while it has none. */
function lastTime(thread: HomeThread): string {
  return thread.last_post_at ?? thread.created_at;
}

function ThreadRow({ thread, open }: { thread: HomeThread; open: boolean }) {
  const { workspaceId, projectId, trackId } = useTrack();
  const posts = thread.post_count ?? 0;
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/tracks/$trackId/threads/$threadId"
        params={{ workspaceId, projectId, trackId, threadId: thread.id }}
        activeProps={{}}
        aria-current={open ? "page" : undefined}
        className={cn(rowClass({ link: true, inset: true }), open && "bg-selected hover:bg-selected")}
      >
        <RowLead>
          <MessageSquare aria-hidden strokeWidth={2} className="size-(--icon-sm) text-ink-3" />
        </RowLead>
        <span className="min-w-0 flex-1 truncate font-medium text-ink">{thread.question}</span>
        {thread.sealed && (
          <span className="inline-flex items-center gap-1 text-meta text-ink-3">
            <Lock aria-hidden strokeWidth={2.25} className="size-3" />
            Sealed
          </span>
        )}
        {thread.conclusion && (
          <span className="inline-flex items-center gap-1 text-meta text-state-done-ink max-md:hidden">
            <CheckCircle2 aria-hidden strokeWidth={2.25} className="size-3" />
            Concluded
          </span>
        )}
        <span className="w-20 shrink-0 text-meta text-ink-3 max-md:hidden">
          {posts} {posts === 1 ? "post" : "posts"}
        </span>
        <span title={new Date(lastTime(thread)).toLocaleString()} className="w-20 shrink-0 text-right text-meta whitespace-nowrap text-ink-3 max-md:w-auto">
          {timeAgo(lastTime(thread))}
        </span>
      </Link>
    </li>
  );
}

/** The track's threads, and the one `threadId` names open under them. */
export function TrackThreads({ threadId }: { threadId?: string }) {
  const { threads, home } = useTrack();
  const sorted = [...threads].sort((a, b) => lastTime(b).localeCompare(lastTime(a)));
  const open = sorted.filter((thread) => !thread.conclusion);
  const concluded = sorted.filter((thread) => thread.conclusion);
  if (!home) return <div aria-hidden className="h-40 animate-pulse rounded-card bg-frame" />;
  return (
    <div>
      <p className="mb-3 flex min-h-8 items-center text-ink-3">
        <span>
          <span className="font-medium text-ink">{open.length} open</span> · {concluded.length} concluded
        </span>
      </p>
      {threads.length === 0 ? (
        <EmptyLine>No threads yet. Agents open a thread when a question needs a few voices; the answer becomes a decision.</EmptyLine>
      ) : (
        <div>
          {(
            [
              ["Open threads", "Open", open, <MessageSquare key="icon" aria-hidden strokeWidth={2.25} className="size-(--icon-sm) text-ink-2" />],
              ["Concluded threads", "Concluded", concluded, <CheckCircle2 key="icon" aria-hidden strokeWidth={2.25} className="size-(--icon-sm) text-state-done" />],
            ] as const
          ).map(([label, title, list, icon]) =>
            list.length === 0 ? null : (
              <section key={label} aria-label={label} className="mb-2">
                <h2>
                  <Band icon={icon} count={list.length}>
                    {title}
                  </Band>
                </h2>
                <ul className="border-b border-line [&>li:first-child>a]:border-t-0">
                  {list.map((thread) => (
                    <ThreadRow key={thread.id} thread={thread} open={thread.id === threadId} />
                  ))}
                </ul>
              </section>
            ),
          )}
        </div>
      )}
      {threadId && <OpenThread key={threadId} threadId={threadId} />}
    </div>
  );
}

/** One thread, followed live: its question, the conclusion once written, the posts oldest first and the form to post. `#post-<id>` scrolls to a post. */
function OpenThread({ threadId }: { threadId: string }) {
  const { workspaceId, projectId } = useTrack();
  const hash = useLocation({ select: (location) => location.hash });
  const load = useCallback(() => loadThread(workspaceId, projectId, threadId), [workspaceId, projectId, threadId]);
  const { value: thread, failed, reload } = useReloading(load);
  useProjectEvents(projectId, reload);
  useVisit(projectId, thread ? (thread.track_id ?? "app") : undefined, undefined, thread ? threadId : undefined);
  const wanted = hash.startsWith("post-") ? hash.slice("post-".length) : null;
  const shown = thread?.posts?.some((post) => post.id === wanted) ? wanted : null;
  useEffect(() => {
    if (shown) document.getElementById(`post-${shown}`)?.scrollIntoView({ block: "center" });
  }, [shown]);

  if (failed && !thread) {
    return (
      <p role="alert" className="mt-8 text-danger-ink">
        The thread could not be loaded. Try again in a moment.
      </p>
    );
  }
  if (!thread) return <div aria-hidden className="mt-8 h-32 animate-pulse rounded-card bg-frame" />;
  const status = thread.conclusion ? "Concluded" : "Open";
  return (
    <article aria-label={thread.question} className="mt-8 border-t border-line pt-6">
      <h2 className="text-section font-semibold tracking-[-0.006em]">{thread.question}</h2>
      <p className="mt-2 flex flex-wrap items-center gap-1.5 text-meta text-ink-3">
        <span
          className={cn(
            "inline-flex h-5 items-center rounded-badge px-1.5 font-medium",
            thread.conclusion ? "bg-hover text-state-done-ink" : "bg-brand-tint text-brand-ink",
          )}
        >
          {status}
        </span>
        {thread.sealed && (
          <span className="inline-flex h-5 items-center gap-1 rounded-badge bg-hover px-1.5 font-medium text-ink-2">
            <Lock aria-hidden strokeWidth={2.25} className="size-3" />
            Sealed
          </span>
        )}
        <span>Started {shortDate(thread.created_at)}</span>
        {thread.run_tag && (
          <span className="whitespace-nowrap">
            · run <span className="font-mono text-ink-2">{thread.run_tag}</span>
          </span>
        )}
      </p>
      {thread.conclusion && (
        <section aria-label="Conclusion" className="mt-4 rounded-control bg-frame px-3 py-2.5">
          <h3 className="text-meta font-medium text-ink-3">Conclusion</h3>
          <p className="mt-0.5 whitespace-pre-wrap text-ink">{thread.conclusion}</p>
        </section>
      )}
      <div className="mt-5 space-y-6">
        <Posts workspaceId={workspaceId} projectId={projectId} posts={thread.posts ?? []} highlighted={shown} />
        <Composer workspaceId={workspaceId} projectId={projectId} threadId={threadId} onPosted={reload} />
      </div>
    </article>
  );
}
