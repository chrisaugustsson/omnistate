import { Link } from "@tanstack/react-router";
import { Crosshair, Link2, Paperclip } from "lucide-react";
import type { PostBody } from "@/api/generated";
import { authorOf, useMemberNames } from "@/components/project/project-data";
import { Avatar } from "@/components/shell/avatar";
import { timeAgo } from "@/lib/format";
import { useMe } from "@/lib/use-workspace";
import { cn } from "@/lib/utils";

/** Styles for a post's body, Markdown the API has rendered to safe HTML. */
const PROSE =
  "space-y-2 text-[13.5px] leading-relaxed text-muted-foreground [&_a]:text-foreground [&_a]:underline [&_a]:underline-offset-4 [&_code]:rounded [&_code]:bg-secondary [&_code]:px-1 [&_code]:py-px [&_code]:font-mono [&_code]:text-[12px] [&_h1]:font-medium [&_h1]:text-foreground [&_h2]:font-medium [&_h2]:text-foreground [&_h3]:font-medium [&_h3]:text-foreground [&_ol]:list-decimal [&_ol]:space-y-1 [&_ol]:pl-5 [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-secondary [&_pre]:p-3 [&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_strong]:font-medium [&_strong]:text-foreground [&_ul]:list-disc [&_ul]:space-y-1 [&_ul]:pl-5 [&_blockquote]:border-l-2 [&_blockquote]:pl-3";

const CHIP =
  "inline-flex max-w-full items-center gap-1 rounded-md bg-secondary px-1.5 py-0.5 text-[11px] text-muted-foreground ring-1 ring-border ring-inset";

/** Who wrote a post, as the app names authors elsewhere: "You", a member's name, or "Architect agent" with its model as quiet meta. */
type PostWriter = { name: string; agent: boolean; key: string; model: string | null };

function postWriter(author: Record<string, unknown>, names: ReadonlyMap<string, string> | null, me: string | null): PostWriter {
  const who = authorOf(author, names);
  if (!who.agent) return { ...who, name: author.user_id === me ? "You" : who.name, model: null };
  return { ...who, model: typeof author.model === "string" && author.model ? author.model : null };
}

/** The thread's posts, oldest first; the one named by `highlighted` stands out. */
export function Posts({
  workspaceId,
  projectId,
  posts,
  highlighted,
}: {
  workspaceId: string;
  projectId: string;
  posts: PostBody[];
  highlighted: string | null;
}) {
  const names = useMemberNames(workspaceId);
  const me = useMe(workspaceId);
  if (posts.length === 0) return <p className="text-sm text-muted-foreground">No posts yet.</p>;
  return (
    <ol className="relative before:absolute before:top-3 before:bottom-3 before:left-[13px] before:w-px before:bg-border">
      {posts.map((post) => (
        <PostItem
          key={post.id}
          workspaceId={workspaceId}
          projectId={projectId}
          post={post}
          writer={postWriter(post.author, names, me)}
          highlighted={post.id === highlighted}
        />
      ))}
    </ol>
  );
}

function PostItem({
  workspaceId,
  projectId,
  post,
  writer,
  highlighted,
}: {
  workspaceId: string;
  projectId: string;
  post: PostBody;
  writer: PostWriter;
  highlighted: boolean;
}) {
  const human = post.author.kind === "human";
  return (
    <li id={`post-${post.id}`} className="relative flex scroll-mt-6 gap-3 pb-6 last:pb-0">
      <Avatar name={writer.name} agent={writer.agent} tintKey={writer.key} size="lg" className="relative z-10 ring-4 ring-sheet" />
      <div
        className={cn(
          "min-w-0 flex-1 rounded-lg",
          human && "-mt-1.5 bg-signal-soft/60 px-3 py-2 ring-1 ring-signal/10 ring-inset",
          highlighted && "bg-accent/70 px-3 py-2 ring-2 ring-signal/40",
        )}
        data-highlighted={highlighted || undefined}
      >
        <div className="flex min-h-7 flex-wrap items-center gap-x-2 gap-y-1">
          <span className="font-medium text-ink">{writer.name}</span>
          <span className="text-meta text-ink-3">
            {writer.model && <>{writer.model} · </>}
            <time dateTime={post.created_at} title={new Date(post.created_at).toLocaleString()}>
              {timeAgo(post.created_at)}
            </time>
          </span>
        </div>
        <h3 className="mt-1 text-[14px] leading-snug font-medium text-foreground">{post.summary}</h3>
        {/* The API has already made the body safe: Markdown without raw HTML. */}
        <div className={cn("mt-1.5", PROSE)} dangerouslySetInnerHTML={{ __html: post.body_html }} />
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
