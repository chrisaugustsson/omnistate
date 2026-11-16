import { createFileRoute, Link } from "@tanstack/react-router";
import { PartyPopper } from "lucide-react";
import type { CommentBody, HomeDoc, HomeResponse } from "@/api/generated";
import { AgentChip } from "@/components/agent-chip";
import { PILLARS, trackNames, waitingCount } from "@/components/project/items";
import { SectionPage } from "@/components/project/section-page";
import { ElementSnapshot } from "@/components/viewer/comments";
import { authorName, timeAgo } from "@/lib/format";
import { useInboxScope } from "@/lib/inbox-scope";
import { cn } from "@/lib/utils";

// The project's inbox: the comments whose latest reply is an agent's, and
// nothing else, grouped by scope (the app, or a track by its name). The group
// with the newest such reply comes first, and so does the newest reply in each
// group. Replying to a comment or deleting it takes it off the list.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/inbox")({
  component: Inbox,
});

/** A comment that waits, with the agent reply that makes it wait and the doc it is on. */
type Entry = {
  comment: CommentBody;
  reply: CommentBody["replies"][number];
  at: number;
  doc: HomeDoc | undefined;
};

type Group = { track: string | null; name: string; at: number; entries: Entry[] };

/** The waiting comments by scope, newest agent reply first in the groups and in each group. */
function groupsOf(home: HomeResponse): Group[] {
  const names = trackNames(home);
  const docs = new Map(home.docs.map((doc) => [doc.id, doc]));
  const groups = new Map<string | null, Group>();
  for (const comment of home.waiting.comments) {
    const reply = comment.replies.at(-1);
    if (!reply) continue;
    const track = comment.track_id ?? null;
    const entry: Entry = { comment, reply, at: new Date(reply.created_at).getTime(), doc: docs.get(comment.doc_id) };
    const group = groups.get(track) ?? {
      track,
      name: track ? (names.get(track) ?? "Track") : "App",
      at: 0,
      entries: [],
    };
    group.entries.push(entry);
    group.at = Math.max(group.at, entry.at);
    groups.set(track, group);
  }
  for (const group of groups.values()) group.entries.sort((a, b) => b.at - a.at);
  return Array.from(groups.values()).sort((a, b) => b.at - a.at);
}

function Inbox() {
  const { workspaceId, projectId } = Route.useParams();
  const { everyone, setEveryone, scoped } = useInboxScope(workspaceId);

  return (
    <SectionPage
      workspaceId={workspaceId}
      projectId={projectId}
      title="Inbox"
      description={(all) => {
        const home = scoped(all);
        if (!home) return null;
        const count = waitingCount(home);
        return count
          ? `${count} ${count === 1 ? "comment has" : "comments have"} an agent's reply waiting on you. Reply, or delete the comment when you're done with it.`
          : "Nothing waits for you. Comments that an agent has replied to show up here.";
      }}
    >
      {(all) => {
        const home = scoped(all);
        if (!home) return null;
        const groups = groupsOf(home);
        const toggle = (
          <label className="flex items-center gap-2 text-sm text-muted-foreground">
            <input type="checkbox" checked={everyone} onChange={(event) => setEveryone(event.target.checked)} />
            Show everyone's tracks
          </label>
        );
        if (groups.length === 0) {
          return (
            <div className="space-y-6">
              {toggle}
              <div className="rounded-xl border border-dashed px-6 py-14 text-center">
                <PartyPopper className="mx-auto size-6 text-muted-foreground" />
                <p className="mt-3 text-sm font-medium">You're all caught up</p>
                <p className="mt-1 text-sm text-muted-foreground">Agents keep working; new replies appear here live.</p>
              </div>
            </div>
          );
        }
        return (
          <div className="space-y-10">
            {toggle}
            {groups.map((group) => (
              <section key={group.track ?? "app"} aria-label={group.name}>
                <div className="flex items-baseline gap-2.5 border-b pb-2.5">
                  <h2 className="text-[15px] font-semibold tracking-tight">
                    {group.track ? (
                      <Link
                        to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
                        params={{ workspaceId, projectId, trackId: group.track }}
                        className="hover:underline hover:underline-offset-4"
                      >
                        {group.name}
                      </Link>
                    ) : (
                      <Link
                        to="/w/$workspaceId/projects/$projectId/app"
                        params={{ workspaceId, projectId }}
                        className="hover:underline hover:underline-offset-4"
                      >
                        {group.name}
                      </Link>
                    )}
                  </h2>
                  <span className="ml-auto text-xs text-muted-foreground tabular-nums">
                    {group.entries.length} waiting
                  </span>
                </div>
                <ul>
                  {group.entries.map((entry) => (
                    <li key={entry.comment.id} className="border-b">
                      <InboxItem workspaceId={workspaceId} projectId={projectId} entry={entry} />
                    </li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        );
      }}
    </SectionPage>
  );
}

function InboxItem({ workspaceId, projectId, entry }: { workspaceId: string; projectId: string; entry: Entry }) {
  const { comment, reply, doc } = entry;
  const pillar = PILLARS.find((candidate) => candidate.pillar === doc?.pillar)?.title ?? doc?.pillar;
  const clamp = "line-clamp-2 break-words whitespace-pre-wrap";
  return (
    <Link
      to="/w/$workspaceId/projects/$projectId/docs/$docId"
      params={{ workspaceId, projectId, docId: comment.doc_id }}
      search={{ comment: comment.id }}
      className={cn(
        "grid grid-cols-[minmax(0,1fr)_auto] gap-x-5 gap-y-2 px-2 py-4 text-sm transition-colors",
        "hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none",
      )}
    >
      <span className="flex min-w-0 items-center gap-2 text-[13px] font-medium">
        {pillar && (
          <span className="shrink-0 rounded-md bg-secondary px-1.5 py-px text-[11px] text-muted-foreground ring-1 ring-border ring-inset">
            {pillar}
          </span>
        )}
        <span className="truncate">{doc?.title}</span>
      </span>
      <time
        dateTime={reply.created_at}
        title={new Date(reply.created_at).toLocaleString()}
        className="text-xs whitespace-nowrap text-muted-foreground"
      >
        {timeAgo(reply.created_at)}
      </time>
      {comment.orphaned && (
        <span className="col-span-full">
          <ElementSnapshot kind={doc?.kind === "html" ? "html" : "design"} snapshot={comment.snapshot ?? ""} />
        </span>
      )}
      <span className={cn("col-span-full border-l-2 pl-3 text-muted-foreground", clamp)}>{comment.text}</span>
      <span className="col-span-full flex min-w-0 items-start gap-2.5">
        <AgentChip name={authorName(reply.author)} className="shrink-0" />
        <span className={cn("min-w-0", clamp)}>{reply.text}</span>
      </span>
    </Link>
  );
}
