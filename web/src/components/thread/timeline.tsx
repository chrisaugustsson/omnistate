import { Link } from "@tanstack/react-router";
import { CornerDownRight, Crosshair, Link2, Lock, Paperclip, Reply } from "lucide-react";
import type { PhaseBody, PostBody, ThreadBody } from "@/api/generated";
import { AgentAvatar } from "@/components/agent-chip";
import { Badge } from "@/components/ui/badge";
import { authorName, timeAgo } from "@/lib/format";
import { cn } from "@/lib/utils";

type Variant = "default" | "secondary" | "outline" | "amber" | "green" | "violet" | "blue" | "red";

/** The colour of each post type. A directive is the human's, in ink. */
const TYPE_VARIANTS: Record<string, Variant> = {
  directive: "default",
  note: "outline",
  finding: "blue",
  position: "secondary",
  proposal: "secondary",
  objection: "red",
  support: "green",
  question: "amber",
  verdict: "violet",
};

/** A post's type as a small coloured pill; a post of a thread without a change set has none. */
export function PostTypeBadge({ type }: { type?: string | null }) {
  if (!type) return null;
  return (
    <Badge variant={TYPE_VARIANTS[type] ?? "outline"} className="px-1.5 py-0 text-[11px] leading-[18px]">
      {type}
    </Badge>
  );
}

/** Styles for a post's body, Markdown the API has rendered to safe HTML. */
const PROSE =
  "space-y-2 text-[13.5px] leading-relaxed text-muted-foreground [&_a]:text-foreground [&_a]:underline [&_a]:underline-offset-4 [&_code]:rounded [&_code]:bg-secondary [&_code]:px-1 [&_code]:py-px [&_code]:font-mono [&_code]:text-[12px] [&_h1]:font-medium [&_h1]:text-foreground [&_h2]:font-medium [&_h2]:text-foreground [&_h3]:font-medium [&_h3]:text-foreground [&_ol]:list-decimal [&_ol]:space-y-1 [&_ol]:pl-5 [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-secondary [&_pre]:p-3 [&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_strong]:font-medium [&_strong]:text-foreground [&_ul]:list-disc [&_ul]:space-y-1 [&_ul]:pl-5 [&_blockquote]:border-l-2 [&_blockquote]:pl-3";

const CHIP =
  "inline-flex max-w-full items-center gap-1 rounded-md bg-secondary px-1.5 py-0.5 text-[11px] text-muted-foreground ring-1 ring-border ring-inset";

/** The posts grouped by phase, each phase a section with its posts as a timeline. */
export function Phases({
  workspaceId,
  projectId,
  thread,
  summariesOnly,
  onReply,
}: {
  workspaceId: string;
  projectId: string;
  thread: ThreadBody;
  summariesOnly: boolean;
  onReply: (post: PostBody) => void;
}) {
  const posts = thread.posts ?? [];
  const phases = thread.phases ?? [];
  const list = (inList: PostBody[]) => (
    <ol className="relative before:absolute before:top-3 before:bottom-3 before:left-[13px] before:w-px before:bg-border">
      {inList.map((post) => (
        <PostItem
          key={post.id}
          workspaceId={workspaceId}
          projectId={projectId}
          post={post}
          posts={posts}
          summariesOnly={summariesOnly}
          onReply={onReply}
        />
      ))}
    </ol>
  );
  // A thread without a change set has no phases: its posts are one timeline.
  if (phases.length === 0) {
    return posts.length === 0 ? (
      <p className="text-sm text-muted-foreground">No posts yet.</p>
    ) : (
      list(posts)
    );
  }
  return (
    <div className="space-y-10">
      {phases.map((phase) => {
        const inPhase = posts.filter((post) => post.phase === phase.number);
        return (
          <section key={phase.number} aria-labelledby={`phase-${phase.number}`} className="space-y-4">
            <PhaseHeader phase={phase} count={inPhase.length} />
            {inPhase.length === 0 ? (
              <p className="pl-10 text-sm text-muted-foreground">No posts in this phase yet.</p>
            ) : (
              list(inPhase)
            )}
          </section>
        );
      })}
    </div>
  );
}

function PhaseHeader({ phase, count }: { phase: PhaseBody; count: number }) {
  const posters = phase.posted_by.map((poster) => poster.role ?? poster.run ?? "agent");
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b pb-3">
      <h2 id={`phase-${phase.number}`} className="text-[15px] font-semibold tracking-tight">
        Phase {phase.number}
      </h2>
      {phase.sealed && (
        <Badge variant="violet" className="px-1.5 py-0 text-[11px] leading-[18px]">
          <Lock />
          Sealed
        </Badge>
      )}
      <span className="text-xs text-muted-foreground tabular-nums">
        {count} {count === 1 ? "post" : "posts"} · {timeAgo(phase.started_at)}
      </span>
      {posters.length > 0 && (
        <span className="ml-auto flex items-center" title={`Posted by ${posters.join(", ")}`}>
          <span className="sr-only">Posted by {posters.join(", ")}</span>
          {posters.map((name, index) => (
            <AgentAvatar
              key={`${name}-${index}`}
              name={name}
              className={cn("size-6 text-[9px] ring-2 ring-background", index > 0 && "-ml-1.5")}
            />
          ))}
        </span>
      )}
    </div>
  );
}

function PostItem({
  workspaceId,
  projectId,
  post,
  posts,
  summariesOnly,
  onReply,
}: {
  workspaceId: string;
  projectId: string;
  post: PostBody;
  posts: PostBody[];
  summariesOnly: boolean;
  onReply: (post: PostBody) => void;
}) {
  const author = authorName(post.author);
  const human = post.author.kind === "human";
  const repliesTo = post.links.flatMap((link) =>
    link.kind === "post" && typeof link.post_id === "string"
      ? posts.filter((other) => other.id === link.post_id)
      : [],
  );
  return (
    <li className="group relative flex gap-3 pb-6 last:pb-0">
      <AgentAvatar
        name={author}
        className={cn(
          "relative z-10 size-7 text-[10px] ring-4 ring-background",
          human && "bg-signal! text-signal-foreground!",
        )}
      />
      <div className={cn("min-w-0 flex-1 rounded-lg", human && "-mt-1.5 bg-signal-soft/60 px-3 py-2 ring-1 ring-signal/10 ring-inset")}>
        <div className="flex min-h-7 flex-wrap items-center gap-x-2 gap-y-1">
          <span className={cn("font-mono text-[12px]", human ? "font-medium text-foreground" : "text-foreground/80")}>
            {author}
          </span>
          <PostTypeBadge type={post.type} />
          {typeof post.confidence === "number" && (
            <span className="text-[11px] text-muted-foreground tabular-nums" title="How sure the agent is">
              {Math.round(post.confidence * 100)}% sure
            </span>
          )}
          <span className="text-[11px] text-muted-foreground">{timeAgo(post.created_at)}</span>
          <button
            type="button"
            onClick={() => onReply(post)}
            className="ml-auto inline-flex h-6 items-center gap-1 rounded-md px-1.5 text-[12px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
          >
            <Reply className="size-3.5" />
            Reply
          </button>
        </div>
        {repliesTo.map((other) => (
          <p key={other.id} className="mt-0.5 flex min-w-0 items-center gap-1 text-[12px] text-muted-foreground">
            <CornerDownRight className="size-3 shrink-0" />
            <span className="truncate">{`In reply to ${authorName(other.author)}: ${other.summary}`}</span>
          </p>
        ))}
        <p className="mt-1 text-[14px] leading-snug font-medium text-foreground">{post.summary}</p>
        {!summariesOnly && (
          // The API has already made the body safe: Markdown without raw HTML.
          <div className={cn("mt-1.5", PROSE)} dangerouslySetInnerHTML={{ __html: post.body_html }} />
        )}
        <PostRefs workspaceId={workspaceId} projectId={projectId} post={post} />
      </div>
    </li>
  );
}

/** The nodes, web pages and attachments a post points at, as small chips; a node opens in the doc's viewer. */
function PostRefs({ workspaceId, projectId, post }: { workspaceId: string; projectId: string; post: PostBody }) {
  const nodes = post.links.flatMap((link) =>
    link.kind === "node" && typeof link.doc_id === "string" && typeof link.address === "string"
      ? [{ docId: link.doc_id, address: link.address }]
      : [],
  );
  const urls = post.links.flatMap((link) =>
    typeof link.url === "string" && /^https?:\/\//.test(link.url) ? [link.url] : [],
  );
  if (nodes.length === 0 && urls.length === 0 && post.attachments.length === 0) return null;
  return (
    <ul className="mt-2.5 flex flex-wrap gap-1.5">
      {nodes.map(({ docId, address }) => (
        <li key={`${docId}/${address}`} className="max-w-full">
          <Link
            to="/w/$workspaceId/projects/$projectId/docs/$docId"
            params={{ workspaceId, projectId, docId }}
            search={{ node: address }}
            className={cn(CHIP, "transition-colors hover:bg-accent hover:text-foreground")}
          >
            <Crosshair className="size-3 shrink-0" />
            <span className="truncate font-mono">Node {address}</span>
          </Link>
        </li>
      ))}
      {urls.map((url) => (
        <li key={url} className="max-w-full">
          <a href={url} target="_blank" rel="noreferrer" className={cn(CHIP, "transition-colors hover:bg-accent hover:text-foreground")}>
            <Link2 className="size-3 shrink-0" />
            <span className="truncate">{url.replace(/^https?:\/\//, "")}</span>
          </a>
        </li>
      ))}
      {post.attachments.map((attachment) => (
        <li key={attachment} className={CHIP}>
          <Paperclip className="size-3 shrink-0" />
          <span className="truncate font-mono">{attachment.replace(/^attachment:/, "").slice(0, 8)}</span>
        </li>
      ))}
    </ul>
  );
}
