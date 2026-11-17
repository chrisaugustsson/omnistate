import { useState, type FormEvent } from "react";
import { SendHorizontal } from "lucide-react";
import { createPost } from "@/api/threads";
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
    <form onSubmit={submit} aria-label="Write a post">
      <div className="min-w-0 overflow-hidden rounded-card border border-line-strong bg-sheet transition-shadow focus-within:border-brand focus-within:shadow-focus">
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
          className="block w-full bg-transparent px-3 pt-2.5 font-medium text-ink outline-none placeholder:font-normal placeholder:text-ink-3"
        />
        <label htmlFor="post-body" className="sr-only">
          Body
        </label>
        <textarea
          id="post-body"
          value={body}
          onChange={(event) => setBody(event.target.value)}
          placeholder="Details, in Markdown"
          className="block min-h-16 w-full resize-y bg-transparent px-3 pt-1 pb-2 text-ink outline-none placeholder:text-ink-3"
        />
        <div className="flex items-center gap-2 border-t border-line bg-frame py-1.5 pr-1.5 pl-3">
          <span className="min-w-0 flex-1 truncate text-meta text-ink-3">Agents on this track read new posts on their next run.</span>
          <Button type="submit" size="sm" disabled={busy || !ready} className="h-(--control-h) rounded-control px-2.5">
            Post
            <SendHorizontal className="size-3.5" />
          </Button>
        </div>
      </div>
    </form>
  );
}
