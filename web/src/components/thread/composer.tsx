import { useState, type FormEvent } from "react";
import { CornerDownRight, SendHorizontal, X } from "lucide-react";
import { createDirective, createPost, type PostLink } from "@/api/threads";
import type { PostBody } from "@/api/generated";
import { AgentAvatar } from "@/components/agent-chip";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { authorName } from "@/lib/format";
import { Segmented } from "./segmented";

type PostType = "directive" | "note" | "question";

const TYPES: { value: PostType; label: string }[] = [
  { value: "directive", label: "Directive" },
  { value: "note", label: "Note" },
  { value: "question", label: "Question" },
];

/** The form to post a directive, note or question; with `replyTo` the post links to the post it answers. */
export function Composer({
  workspaceId,
  projectId,
  threadId,
  replyTo,
  onCancelReply,
  onPosted,
}: {
  workspaceId: string;
  projectId: string;
  threadId: string;
  replyTo: PostBody | null;
  onCancelReply: () => void;
  onPosted: () => void;
}) {
  const [type, setType] = useState<PostType>("note");
  const [summary, setSummary] = useState("");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!summary.trim() || busy) return;
    setBusy(true);
    setFailed(false);
    const links: PostLink[] = replyTo ? [{ kind: "post", post_id: replyTo.id }] : [];
    try {
      if (type === "directive") {
        await createDirective(workspaceId, projectId, threadId, { summary, body, links });
      } else {
        await createPost(workspaceId, projectId, threadId, { type, summary, body, links });
      }
      setSummary("");
      setBody("");
      onPosted();
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      onSubmit={submit}
      aria-label="Write a post"
      className="flex gap-3"
    >
      <AgentAvatar name="You" className="mt-3 size-7 bg-signal! text-[10px] text-signal-foreground!" />
      <div className="min-w-0 flex-1 overflow-hidden rounded-xl border bg-card shadow-xs transition-shadow focus-within:border-foreground/20 focus-within:shadow-panel">
        <div className="flex min-h-10 flex-wrap items-center gap-2 px-4 pt-3">
          <h2 className="text-[13px] font-semibold">Write a post</h2>
          {replyTo && (
            <span className="inline-flex max-w-full min-w-0 items-center gap-1 rounded-md bg-secondary py-0.5 pr-0.5 pl-1.5 text-[12px] text-muted-foreground ring-1 ring-border ring-inset">
              <CornerDownRight className="size-3 shrink-0" />
              <span className="truncate">
                Replying to {authorName(replyTo.author)}: {replyTo.summary}
              </span>
              <button
                type="button"
                aria-label="Cancel reply"
                onClick={onCancelReply}
                className="ml-0.5 inline-flex size-5 shrink-0 items-center justify-center rounded hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
              >
                <X className="size-3" />
              </button>
            </span>
          )}
        </div>
        {failed && (
          <Alert variant="destructive" className="mx-4 mt-2 w-auto">
            <AlertDescription>The post could not be sent.</AlertDescription>
          </Alert>
        )}
        <label htmlFor="post-summary" className="sr-only">
          Summary
        </label>
        <input
          id="post-summary"
          maxLength={200}
          value={summary}
          onChange={(event) => setSummary(event.target.value)}
          placeholder="Summary: the one line agents read first"
          className="block w-full bg-transparent px-4 pt-2 text-[14px] font-medium outline-none placeholder:font-normal placeholder:text-muted-foreground/70"
        />
        <label htmlFor="post-body" className="sr-only">
          Body
        </label>
        <textarea
          id="post-body"
          value={body}
          onChange={(event) => setBody(event.target.value)}
          placeholder="Details, in Markdown"
          className="block min-h-20 w-full resize-y bg-transparent px-4 pt-1.5 pb-3 text-[13.5px] leading-relaxed outline-none placeholder:text-muted-foreground/70"
        />
        <div className="flex flex-wrap items-center justify-between gap-2 border-t bg-secondary/40 px-3 py-2">
          <Segmented name="post-type" label="Type" options={TYPES} value={type} onChange={setType} />
          <Button type="submit" size="sm" disabled={busy || !summary.trim()}>
            Post
            <SendHorizontal className="size-3.5" />
          </Button>
        </div>
      </div>
    </form>
  );
}
