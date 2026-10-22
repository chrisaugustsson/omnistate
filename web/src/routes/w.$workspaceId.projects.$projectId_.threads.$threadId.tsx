import { useCallback, useEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import {
  approveDecision,
  createDirective,
  createPost,
  loadThread,
  rejectDecision,
  type PostLink,
} from "@/api/threads";
import { DecisionBodySchema, type DecisionBody, type PostBody, type ThreadBody } from "@/api/generated";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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

const SELECT_CLASS =
  "h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";
const TEXTAREA_CLASS =
  "min-h-24 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";

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

function ThreadView() {
  const { workspaceId, projectId, threadId } = Route.useParams();
  const { thread, failed, reload } = useThread(workspaceId, projectId, threadId);
  const [summariesOnly, setSummariesOnly] = useState(false);
  const [replyTo, setReplyTo] = useState<PostBody | null>(null);

  return (
    <main className="mx-auto max-w-5xl space-y-8 p-6">
      <Link
        to="/w/$workspaceId/projects/$projectId"
        params={{ workspaceId, projectId }}
        className="text-sm text-muted-foreground underline-offset-4 hover:underline"
      >
        Back to the project
      </Link>
      {failed && (
        <Alert variant="destructive">
          <AlertDescription>The thread could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {thread && (
        <>
          <header className="space-y-2">
            <h1 className="text-2xl font-semibold">{thread.question}</h1>
            {thread.criteria.length > 0 && (
              <p className="text-sm text-muted-foreground">Criteria: {thread.criteria.join(", ")}</p>
            )}
          </header>
          <DecisionPanel
            workspaceId={workspaceId}
            projectId={projectId}
            decision={parseDecision(thread)}
            posts={thread.posts ?? []}
            onChanged={reload}
          />
          <div className="flex items-center gap-2">
            <input
              id="summaries-only"
              type="checkbox"
              checked={summariesOnly}
              onChange={(event) => setSummariesOnly(event.target.checked)}
            />
            <Label htmlFor="summaries-only">Summaries only</Label>
          </div>
          <Phases thread={thread} summariesOnly={summariesOnly} onReply={setReplyTo} />
          <PostForm
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
        </>
      )}
    </main>
  );
}

function parseDecision(thread: ThreadBody): DecisionBody | null {
  const parsed = DecisionBodySchema.safeParse(thread.decision);
  return parsed.success ? parsed.data : null;
}

function Section({ id, title, badge, children }: { id: string; title: string; badge?: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="space-y-3">
      <div className="flex items-center gap-2">
        <h2 id={id} className="text-lg font-semibold">
          {title}
        </h2>
        {badge && <span className="rounded-full border px-2 py-0.5 text-xs text-muted-foreground">{badge}</span>}
      </div>
      {children}
    </section>
  );
}

/** Who wrote a post or proposed a decision: an agent's role, or the human. */
function authorName(author: Record<string, unknown>): string {
  if (author.kind === "human") return "You";
  return typeof author.role === "string" && author.role ? author.role : "agent";
}

/** The nodes a post points at, as links that open the node in the doc's viewer. */
function NodeLinks({ post }: { post: PostBody }) {
  const { workspaceId, projectId } = Route.useParams();
  const nodes = post.links.flatMap((link) =>
    link.kind === "node" && typeof link.doc_id === "string" && typeof link.address === "string"
      ? [{ docId: link.doc_id, address: link.address }]
      : [],
  );
  if (nodes.length === 0) return null;
  return (
    <ul className="flex flex-wrap gap-x-4 gap-y-1">
      {nodes.map(({ docId, address }) => (
        <li key={`${docId}/${address}`}>
          <Link
            to="/w/$workspaceId/projects/$projectId/docs/$docId"
            params={{ workspaceId, projectId, docId }}
            search={{ node: address }}
            className="font-mono text-xs text-muted-foreground underline underline-offset-4 hover:text-foreground"
          >
            Node {address}
          </Link>
        </li>
      ))}
    </ul>
  );
}

function Phases({
  thread,
  summariesOnly,
  onReply,
}: {
  thread: ThreadBody;
  summariesOnly: boolean;
  onReply: (post: PostBody) => void;
}) {
  const posts = thread.posts ?? [];
  return (
    <div className="space-y-6">
      {thread.phases.map((phase) => (
        <Section
          key={phase.number}
          id={`phase-${phase.number}`}
          title={`Phase ${phase.number}`}
          badge={phase.sealed ? "sealed" : undefined}
        >
          <ul className="divide-y rounded-md border empty:hidden">
            {posts
              .filter((post) => post.phase === phase.number)
              .map((post) => (
                <li key={post.id} className="space-y-1 px-4 py-3 text-sm">
                  <div className="flex items-center justify-between gap-3">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="rounded-full border px-2 py-0.5 text-xs text-muted-foreground">{post.type}</span>
                      <span className="font-medium">{post.summary}</span>
                      <span className="text-xs text-muted-foreground">{authorName(post.author)}</span>
                    </div>
                    <Button variant="ghost" size="sm" onClick={() => onReply(post)}>
                      Reply
                    </Button>
                  </div>
                  {!summariesOnly && (
                    // The API has already made the body safe: Markdown without raw HTML.
                    <div className="space-y-2 text-muted-foreground" dangerouslySetInnerHTML={{ __html: post.body_html }} />
                  )}
                  <NodeLinks post={post} />
                </li>
              ))}
          </ul>
        </Section>
      ))}
    </div>
  );
}

function PostForm({
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
  const [type, setType] = useState<"directive" | "note" | "question">("note");
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
    <form onSubmit={submit} className="space-y-3 rounded-md border p-4" aria-label="Write a post">
      <h2 className="text-lg font-semibold">Write a post</h2>
      {replyTo && (
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          <span>Replying to: {replyTo.summary}</span>
          <Button type="button" variant="ghost" size="sm" onClick={onCancelReply}>
            Cancel reply
          </Button>
        </p>
      )}
      {failed && (
        <Alert variant="destructive">
          <AlertDescription>The post could not be sent.</AlertDescription>
        </Alert>
      )}
      <div className="space-y-1">
        <Label htmlFor="post-type">Type</Label>
        <select
          id="post-type"
          className={SELECT_CLASS}
          value={type}
          onChange={(event) => setType(event.target.value as typeof type)}
        >
          <option value="directive">directive</option>
          <option value="note">note</option>
          <option value="question">question</option>
        </select>
      </div>
      <div className="space-y-1">
        <Label htmlFor="post-summary">Summary</Label>
        <Input id="post-summary" maxLength={200} value={summary} onChange={(event) => setSummary(event.target.value)} />
      </div>
      <div className="space-y-1">
        <Label htmlFor="post-body">Body</Label>
        <textarea
          id="post-body"
          className={TEXTAREA_CLASS}
          value={body}
          onChange={(event) => setBody(event.target.value)}
        />
      </div>
      <Button type="submit" disabled={busy || !summary.trim()}>
        Post
      </Button>
    </form>
  );
}

function DecisionPanel({
  workspaceId,
  projectId,
  decision,
  posts,
  onChanged,
}: {
  workspaceId: string;
  projectId: string;
  decision: DecisionBody | null;
  posts: PostBody[];
  onChanged: () => void;
}) {
  const [rejecting, setRejecting] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  if (!decision) {
    return (
      <Section id="decision" title="Decision">
        <p className="text-sm text-muted-foreground">No decision has been proposed yet.</p>
      </Section>
    );
  }

  async function act(call: () => Promise<unknown>) {
    setBusy(true);
    setFailed(false);
    try {
      await call();
      setRejecting(false);
      setReason("");
      onChanged();
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  const current = decision;
  const relied = current.posts.map((id) => posts.find((post) => post.id === id)).filter((post) => post !== undefined);
  return (
    <Section id="decision" title="Decision" badge={current.status}>
      <div className="space-y-4 rounded-md border p-4 text-sm">
        {failed && (
          <Alert variant="destructive">
            <AlertDescription>The decision could not be changed. Reload and try again.</AlertDescription>
          </Alert>
        )}
        <p className="text-base font-medium">{current.direction}</p>
        <dl className="grid gap-x-4 gap-y-1 sm:grid-cols-[10rem_1fr]">
          <dt className="text-muted-foreground">Chosen option</dt>
          <dd>{current.chosen}</dd>
          <dt className="text-muted-foreground">Proposed by</dt>
          <dd>{authorName(current.author)}</dd>
          <dt className="text-muted-foreground">Criteria</dt>
          <dd>
            <ul className="list-disc pl-4">
              {current.criteria.map((criterion) => (
                <li key={criterion}>{criterion}</li>
              ))}
            </ul>
          </dd>
        </dl>
        <div>
          <h3 className="font-medium">Options</h3>
          <ul className="mt-1 divide-y rounded-md border">
            {current.options.map((option) => (
              <li key={option.name} className="space-y-1 px-3 py-2">
                <span className="font-medium">{option.name}</span>
                <ul className="flex flex-wrap gap-x-4 text-muted-foreground">
                  {Object.entries(option.scores).map(([criterion, score]) => (
                    <li key={criterion}>
                      <span>{criterion}: </span>
                      <span>{String(score)}</span>
                    </li>
                  ))}
                </ul>
                {option.lost_because && <p>Lost because: {option.lost_because}</p>}
              </li>
            ))}
          </ul>
        </div>
        <TextList title="Assumptions" items={current.assumptions} />
        <TextList title="Open questions" items={current.open_questions} />
        {current.dissent.length > 0 && (
          <div>
            <h3 className="font-medium">Dissent</h3>
            <ul className="mt-1 list-disc pl-4">
              {current.dissent.map((entry, index) => (
                <li key={index}>
                  <span className="text-muted-foreground">{entry.by}: </span>
                  <span>{entry.text}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
        {relied.length > 0 && (
          <div>
            <h3 className="font-medium">Relies on</h3>
            <ul className="mt-1 list-disc pl-4">
              {relied.map((post) => (
                <li key={post.id}>
                  <span>{post.summary}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
        {current.rejection_reason && (
          <p>
            <span className="text-muted-foreground">Reason for rejecting: </span>
            <span>{current.rejection_reason}</span>
          </p>
        )}
        {current.status === "proposed" && (
          <div className="space-y-3">
            {rejecting && (
              <div className="space-y-1">
                <Label htmlFor="reject-reason">Reason</Label>
                <textarea
                  id="reject-reason"
                  className={TEXTAREA_CLASS}
                  value={reason}
                  onChange={(event) => setReason(event.target.value)}
                />
              </div>
            )}
            <div className="flex gap-2">
              <Button
                disabled={busy}
                onClick={() => act(() => approveDecision(workspaceId, projectId, current.id))}
              >
                Approve
              </Button>
              {rejecting ? (
                <>
                  <Button
                    variant="destructive"
                    disabled={busy || !reason.trim()}
                    onClick={() => act(() => rejectDecision(workspaceId, projectId, current.id, reason.trim()))}
                  >
                    Reject
                  </Button>
                  <Button variant="ghost" onClick={() => setRejecting(false)}>
                    Cancel
                  </Button>
                </>
              ) : (
                <Button variant="outline" disabled={busy} onClick={() => setRejecting(true)}>
                  Reject
                </Button>
              )}
            </div>
          </div>
        )}
      </div>
    </Section>
  );
}

function TextList({ title, items }: { title: string; items: string[] }) {
  if (!items.length) return null;
  return (
    <div>
      <h3 className="font-medium">{title}</h3>
      <ul className="mt-1 list-disc pl-4">
        {items.map((item, index) => (
          <li key={index}>{item}</li>
        ))}
      </ul>
    </div>
  );
}
