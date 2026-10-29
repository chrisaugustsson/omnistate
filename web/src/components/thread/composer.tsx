import { useState, type FormEvent } from "react";
import { SendHorizontal } from "lucide-react";
import { createPost } from "@/api/threads";
import { AgentAvatar } from "@/components/agent-chip";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";

/** The form to post in a thread: a one-line summary and a Markdown body, both required, sent as they are. */
export function Composer({
  workspaceId,
  projectId,
  threadId,
  onPosted,
}: {
  workspaceId: string;
  projectId: string;
  threadId: string;
  onPosted: () => void;
}) {
  const [summary, setSummary] = useState("");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const ready = summary.trim() !== "" && body.trim() !== "";

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!ready || busy) return;
    setBusy(true);
    setFailed(false);
    try {
      await createPost(workspaceId, projectId, threadId, { summary, body });
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
    <form onSubmit={submit} aria-label="Write a post" className="flex gap-3">
      <AgentAvatar name="You" className="mt-3 size-7 bg-signal! text-[10px] text-signal-foreground!" />
      <div className="min-w-0 flex-1 overflow-hidden rounded-xl border bg-card shadow-xs transition-shadow focus-within:border-foreground/20 focus-within:shadow-panel">
        <div className="flex min-h-10 items-center px-4 pt-3">
          <h2 className="text-[13px] font-semibold">Write a post</h2>
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
        <div className="flex justify-end border-t bg-secondary/40 px-3 py-2">
          <Button type="submit" size="sm" disabled={busy || !ready}>
            Post
            <SendHorizontal className="size-3.5" />
          </Button>
        </div>
      </div>
    </form>
  );
}
