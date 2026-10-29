import { useEffect, useRef, useState, type FormEvent } from "react";
import { MessageSquare, MousePointerClick, Send, Trash2 } from "lucide-react";
import { createDocComment, deleteComment, replyToComment } from "@/api/docs";
import type { CommentBody } from "@/api/generated";
import { AgentChip } from "@/components/agent-chip";
import { Button } from "@/components/ui/button";
import { PaneEmpty, PaneSection } from "@/components/viewer/chrome";
import { THUMBNAIL, drawSnapshot } from "@/design/thumbnail";
import { authorName, timeAgo } from "@/lib/format";
import { cn } from "@/lib/utils";

// The comments of a doc, as the doc viewers and the Inbox show them: a comment
// with its replies, oldest first, a reply box and a Delete button, and, for a
// comment whose element an agent removed, what the element was.

const TEXTAREA_CLASS =
  "block min-h-20 w-full resize-none bg-transparent px-3 pt-2.5 pb-1 text-[13px] leading-relaxed outline-none placeholder:text-muted-foreground";

/** What a snapshot is the snapshot of: a block of an HTML doc, or a node of a design. */
export type SnapshotKind = "html" | "design";

/** How much of an HTML node's text its snapshot shows. */
const SNAPSHOT_TEXT = 80;

/**
 * The tag and the start of the text of the HTML that a snapshot holds, as
 * plain text. The HTML is read into a document of its own that runs nothing
 * and loads nothing, and only its text leaves it.
 */
export function htmlSnapshot(html: string): { tag: string; text: string } | null {
  const element = new DOMParser().parseFromString(html, "text/html").body.firstElementChild;
  if (!element) return null;
  const text = (element.textContent ?? "").replace(/\s+/g, " ").trim();
  return {
    tag: element.tagName.toLowerCase(),
    text: text.length > SNAPSHOT_TEXT ? `${text.slice(0, SNAPSHOT_TEXT).trimEnd()}…` : text,
  };
}

/** The name of the node that a snapshot in tag form holds: the `name` of its first tag. */
export function designSnapshotName(tags: string): string {
  const name = /^\s*<\w+\s+(?:id="[^"]*"\s+)?name="((?:[^"\\]|\\.)*)"/.exec(tags);
  return name ? name[1].replace(/\\(.)/g, "$1") : "";
}

/** A node of a design, drawn by the renderer from its tag form, with its name. */
function DesignSnapshot({ tags }: { tags: string }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [drawn, setDrawn] = useState(false);
  const name = designSnapshotName(tags);
  useEffect(() => {
    const target = canvas.current;
    if (!target) return;
    let cancelled = false;
    setDrawn(false);
    // A node that cannot be drawn keeps its name and no picture.
    drawSnapshot(tags, target).then(
      () => {
        if (!cancelled) setDrawn(true);
      },
      () => undefined,
    );
    return () => {
      cancelled = true;
    };
  }, [tags]);
  return (
    <>
      <canvas
        ref={canvas}
        role="img"
        aria-label={name ? `Picture of ${name}` : "Picture of the element"}
        style={{ width: THUMBNAIL.width / 2, height: THUMBNAIL.height / 2 }}
        className={cn("shrink-0 rounded bg-muted", !drawn && "hidden")}
      />
      {name && <span className="min-w-0 truncate font-medium text-foreground">{name}</span>}
    </>
  );
}

/** A block of an HTML doc as its tag and the start of its text. */
function HtmlSnapshot({ html }: { html: string }) {
  const shown = htmlSnapshot(html);
  if (!shown) return null;
  return (
    <>
      <code className="shrink-0 font-mono text-[11.5px] text-foreground">{`<${shown.tag}>`}</code>
      {shown.text && <span className="min-w-0 truncate">{shown.text}</span>}
    </>
  );
}

/**
 * What the element of a comment was, once an agent removed it: a picture and
 * the name of a design's node, or the tag and the start of the text of an HTML
 * doc's block, with an "Element removed" label. The snapshot is never put into
 * the page as HTML.
 */
export function ElementSnapshot({ kind, snapshot }: { kind: SnapshotKind; snapshot: string }) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-lg border border-dashed p-2 text-[12.5px] text-muted-foreground">
      {snapshot && (kind === "design" ? <DesignSnapshot tags={snapshot} /> : <HtmlSnapshot html={snapshot} />)}
      <span className="ml-auto inline-flex h-5 shrink-0 items-center rounded-md bg-signal-soft px-1.5 text-[11.5px] font-medium text-signal">
        Element removed
      </span>
    </div>
  );
}

/** A time as relative text, with the exact time as its tooltip. */
function When({ at, className }: { at: string; className?: string }) {
  return (
    <time dateTime={at} title={new Date(at).toLocaleString()} className={className}>
      {timeAgo(at)}
    </time>
  );
}

/** A box for a short text, with a button to send it. */
function Composer({
  id,
  label,
  placeholder,
  button,
  failure,
  onSend,
}: {
  id: string;
  label: string;
  placeholder: string;
  button: string;
  failure: string;
  onSend: (text: string) => Promise<void>;
}) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [failed, setFailed] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const said = text.trim();
    if (!said || sending) return;
    setSending(true);
    setFailed(false);
    try {
      await onSend(said);
      setText("");
    } catch {
      setFailed(true);
    } finally {
      setSending(false);
    }
  }

  return (
    <form
      onSubmit={submit}
      className="overflow-hidden rounded-lg border bg-background shadow-xs transition-shadow focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/20"
    >
      <label htmlFor={id} className="sr-only">
        {label}
      </label>
      <textarea
        id={id}
        className={TEXTAREA_CLASS}
        placeholder={placeholder}
        value={text}
        onChange={(event) => setText(event.target.value)}
      />
      {failed && (
        <p role="alert" className="px-3 pb-1 text-[12px] text-destructive">
          {failure}
        </p>
      )}
      <div className="flex items-center justify-end gap-2 px-2 pb-2">
        <Button type="submit" size="sm" className="h-7 gap-1.5 px-2.5 text-[12px]" disabled={sending || !text.trim()}>
          <Send aria-hidden className="size-3" />
          {button}
        </Button>
      </div>
    </form>
  );
}

/**
 * A comment: who wrote it and when, what it says, its replies oldest first,
 * a reply box and a Delete button. A comment whose element was removed shows
 * what the element was. `open` marks the comment that a link opened.
 */
export function CommentCard({
  workspaceId,
  projectId,
  comment,
  kind,
  open,
  onChanged,
}: {
  workspaceId: string;
  projectId: string;
  comment: CommentBody;
  kind: SnapshotKind;
  open?: boolean;
  /** Called when the comment was replied to or deleted, so that the list is read again. */
  onChanged: () => void;
}) {
  const card = useRef<HTMLLIElement>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (open) card.current?.scrollIntoView?.({ block: "nearest" });
  }, [open]);

  async function remove() {
    setFailed(false);
    try {
      await deleteComment(workspaceId, projectId, comment.id);
      onChanged();
    } catch {
      setFailed(true);
    }
  }

  return (
    <li
      ref={card}
      id={`comment-${comment.id}`}
      aria-current={open ? "true" : undefined}
      className={cn("space-y-2 rounded-lg border bg-card p-3 text-[13px]", open && "border-signal ring-[3px] ring-signal/20")}
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <AgentChip name={authorName(comment.author)} className="shrink-0" />
        <When at={comment.created_at} className="text-[11px] text-muted-foreground" />
        <div className="ml-auto flex shrink-0 items-center gap-1.5">
          {comment.waiting_on_you && (
            <span className="rounded-full bg-signal-soft px-1.5 text-[11px] font-medium">Waiting on you</span>
          )}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-6 gap-1 px-1.5 text-[11.5px] text-muted-foreground hover:text-destructive"
            onClick={remove}
          >
            <Trash2 aria-hidden className="size-3" />
            Delete
          </Button>
        </div>
      </div>
      {comment.orphaned && <ElementSnapshot kind={kind} snapshot={comment.snapshot ?? ""} />}
      <p className="leading-relaxed break-words whitespace-pre-wrap">{comment.text}</p>
      {comment.replies.length > 0 && (
        <ul className="space-y-2 border-l-2 pl-2.5">
          {comment.replies.map((reply) => (
            <li key={reply.id} className="space-y-0.5">
              <div className="flex items-center justify-between gap-2">
                <AgentChip name={authorName(reply.author)} className="shrink-0" />
                <When at={reply.created_at} className="text-[11px] text-muted-foreground" />
              </div>
              <p className="leading-relaxed break-words whitespace-pre-wrap text-muted-foreground">{reply.text}</p>
            </li>
          ))}
        </ul>
      )}
      {failed && (
        <p role="alert" className="text-[12px] text-destructive">
          The comment could not be deleted.
        </p>
      )}
      <Composer
        id={`reply-${comment.id}`}
        label="Reply"
        placeholder="Reply…"
        button="Reply"
        failure="The reply could not be saved."
        onSend={async (text) => {
          await replyToComment(workspaceId, projectId, comment.id, text);
          onChanged();
        }}
      />
    </li>
  );
}

/**
 * The comments of the doc viewers: the comments on the selected element with a
 * box for a new one, and, below, the comments whose element an agent removed,
 * which have no pin. `open` is the comment that a link opened.
 */
export function CommentsPanel({
  workspaceId,
  projectId,
  docId,
  kind,
  selected,
  comments,
  removed,
  open,
  noun,
  onChanged,
}: {
  workspaceId: string;
  projectId: string;
  docId: string;
  kind: SnapshotKind;
  selected: string | null;
  /** The comments on the selected element. */
  comments: CommentBody[];
  /** The comments whose element is no longer in the doc. */
  removed: CommentBody[];
  open: string | null;
  /** What a comment is on: a block of a doc, a node of a design. */
  noun: string;
  onChanged: () => void;
}) {
  const card = (comment: CommentBody) => (
    <CommentCard
      key={comment.id}
      workspaceId={workspaceId}
      projectId={projectId}
      comment={comment}
      kind={kind}
      open={comment.id === open}
      onChanged={onChanged}
    />
  );
  return (
    <>
      <PaneSection
        id="comments"
        title="Comments"
        aside={
          selected ? (
            <span className="max-w-32 truncate font-mono text-[10.5px]" title={selected}>
              {selected}
            </span>
          ) : undefined
        }
      >
        {!selected && <PaneEmpty icon={<MousePointerClick />}>Select a {noun} to see its comments.</PaneEmpty>}
        {selected && comments.length === 0 && (
          <PaneEmpty icon={<MessageSquare />}>No comments on this {noun} yet.</PaneEmpty>
        )}
        <ul className="space-y-2 empty:hidden">{comments.map(card)}</ul>
        {selected && (
          <div className="mt-3">
            <Composer
              id="new-comment"
              label="Comment"
              placeholder={`Comment on this ${noun}…`}
              button="Comment"
              failure="The comment could not be saved."
              onSend={async (text) => {
                await createDocComment(workspaceId, projectId, docId, { node: selected, text });
                onChanged();
              }}
            />
          </div>
        )}
      </PaneSection>
      {removed.length > 0 && (
        <PaneSection id="removed-comments" title="Removed elements" aside={removed.length}>
          <ul className="space-y-2">{removed.map(card)}</ul>
        </PaneSection>
      )}
    </>
  );
}
