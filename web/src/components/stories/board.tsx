import { useCallback, useMemo, useState, type FormEvent } from "react";
import { Check } from "lucide-react";
import { ApiError } from "@/api/client";
import { cancelStory, checkCriterion, createStory, loadStories, patchStory } from "@/api/stories";
import type { StoryBody } from "@/api/generated";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { Empty } from "@/components/project/items";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useProjectEvents } from "@/live/context";
import { useProjectName } from "@/lib/use-home";
import { useReloading } from "@/lib/reloading";
import { cn } from "@/lib/utils";

// A feature's board: the five columns of a story's life, a card per story, and
// a filter on priority. The stories are loaded again on every story event of
// the project, so a card changes column as agents move it.

// A column lists its statuses in order: the waiting status first, then the working one.
const COLUMNS = [
  { statuses: ["backlog"], title: "Backlog" },
  { statuses: ["implementing"], title: "Implementing" },
  { statuses: ["implemented", "review"], title: "Review" },
  { statuses: ["reviewed", "testing"], title: "Testing" },
  { statuses: ["done"], title: "Done" },
];
const CANCELLED = { statuses: ["cancelled"], title: "Cancelled" };
const PRIORITIES = ["all", "1", "2", "3"];

const closed = (story: StoryBody) => story.status === "done" || story.status === "cancelled";
const unresolved = (story: StoryBody) => story.findings.filter((finding) => finding.resolved !== true).length;
const toCheck = (story: StoryBody) => story.criteria.filter((criterion) => criterion.ticked && criterion.pending).length;

/** A small count or flag on a card; the tone says how much it asks of the reviewer. */
function Badge({ tone, children }: { tone: "warning" | "neutral" | "accent"; children: string }) {
  return (
    <span
      className={cn(
        "rounded-full px-1.5 text-[11px] leading-5 tabular-nums",
        tone === "warning" && "bg-amber-100 text-amber-900 dark:bg-amber-950 dark:text-amber-200",
        tone === "neutral" && "bg-muted text-muted-foreground",
        tone === "accent" && "bg-signal text-signal-foreground",
      )}
    >
      {children}
    </span>
  );
}

type Steer = {
  stories: StoryBody[];
  /** Messages shown on cards by story id, kept here so they survive a card changing column. */
  messages: Map<string, string>;
  /** Runs a write on a story; a 409 reloads and sets the message to show on its card. */
  run: (story: StoryBody, write: () => Promise<StoryBody>) => Promise<void>;
  patch: (story: StoryBody, body: { priority?: number; blocked_by?: string[] }) => Promise<StoryBody>;
  cancel: (story: StoryBody) => Promise<StoryBody>;
  check: (story: StoryBody, criterion: number) => Promise<StoryBody>;
};

const CONFLICT = "Someone changed this story; it has been reloaded";

function Card({ story, byId, steer }: { story: StoryBody; byId: Map<string, StoryBody>; steer: Steer }) {
  const holder = story.holder as { run?: string; stage?: string } | null | undefined;
  const findings = unresolved(story);
  const check = toCheck(story);
  const isClosed = closed(story);
  const message = steer.messages.get(story.id);
  const [editing, setEditing] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [checking, setChecking] = useState(false);
  const write = async (action: () => Promise<StoryBody>) => {
    await steer.run(story, action);
  };
  const others = steer.stories.filter((other) => other.id !== story.id);
  return (
    <li className="space-y-2 rounded-xl border bg-card px-3 py-2.5 text-sm">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span className="font-medium tabular-nums">{story.key}</span>
        {(story.status === "implemented" || story.status === "reviewed") && <Badge tone="neutral">Waiting</Badge>}
        {story.priority === 0 ? (
          <span className="ml-auto tabular-nums">P0</span>
        ) : (
          <select
            aria-label={`${story.key} priority`}
            value={story.priority}
            disabled={isClosed}
            onChange={(event) => write(() => steer.patch(story, { priority: Number(event.target.value) }))}
            className="ml-auto h-7 rounded-md border border-input bg-background px-1 text-xs tabular-nums"
          >
            {[1, 2, 3].map((value) => (
              <option key={value} value={value}>{`P${value}`}</option>
            ))}
          </select>
        )}
      </div>
      <p className="font-medium">{story.title}</p>
      {holder && (
        <p className="text-xs text-muted-foreground">
          {holder.stage} · {holder.run}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-1.5">
        {story.blocked && <Badge tone="warning">Blocked</Badge>}
        {findings > 0 && <Badge tone="neutral">{`${findings} findings`}</Badge>}
        {check > 0 && (
          <button type="button" aria-expanded={checking} onClick={() => setChecking(!checking)}>
            <Badge tone="accent">{`${check} to check`}</Badge>
          </button>
        )}
      </div>
      {checking && check > 0 && (
        <ul className="space-y-1 text-xs">
          {story.criteria.map((criterion, index) =>
            criterion.ticked && criterion.pending ? (
              <li key={index} className="flex items-center gap-2">
                <span className="min-w-0 flex-1">{criterion.text}</span>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => write(() => steer.check(story, index))}
                >
                  Checked
                </Button>
              </li>
            ) : null,
          )}
        </ul>
      )}
      {story.blocked_by.length > 0 && (
        <ul className="flex flex-wrap gap-x-2 gap-y-1 text-xs text-muted-foreground">
          {story.blocked_by.map((id) => {
            const blocker = byId.get(id);
            return (
              <li key={id} className="inline-flex items-center gap-0.5">
                {blocker?.key ?? "?"}
                {blocker && closed(blocker) && <Check aria-label="done" className="size-3" />}
              </li>
            );
          })}
        </ul>
      )}
      {message && (
        <p role="alert" className="text-xs text-destructive">
          {message}
        </p>
      )}
      <div className="flex flex-wrap gap-1.5">
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={isClosed}
          aria-expanded={editing}
          onClick={() => setEditing(!editing)}
        >
          Edit blockers
        </Button>
        {!isClosed && !confirming && (
          <Button type="button" size="sm" variant="ghost" onClick={() => setConfirming(true)}>
            Cancel story
          </Button>
        )}
      </div>
      {editing && !isClosed && (
        <ul className="space-y-1 text-xs">
          {others.map((other) => (
            <li key={other.id}>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={story.blocked_by.includes(other.id)}
                  onChange={(event) =>
                    write(() =>
                      steer.patch(story, {
                        blocked_by: event.target.checked
                          ? [...story.blocked_by, other.id]
                          : story.blocked_by.filter((id) => id !== other.id),
                      }),
                    )
                  }
                />
                {other.key}
              </label>
            </li>
          ))}
        </ul>
      )}
      {confirming && !isClosed && (
        <div className="flex items-center gap-2 text-xs">
          <span>{`Cancel ${story.key}?`}</span>
          <Button
            type="button"
            size="sm"
            variant="destructive"
            onClick={async () => {
              setConfirming(false);
              await write(() => steer.cancel(story));
            }}
          >
            Confirm
          </Button>
          <Button type="button" size="sm" variant="ghost" onClick={() => setConfirming(false)}>
            Keep
          </Button>
        </div>
      )}
    </li>
  );
}

const BROWSER_LINE = "Verify in browser using dev-browser skill";

/** The criteria of the form's textarea: a line each, trimmed, blank lines skipped. */
function criteriaOf(lines: string) {
  return lines
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
    .map((text) => ({ text, human_check: text.includes("(human check)") || text.endsWith(BROWSER_LINE) }));
}

function AddStory({ onAdd, onClose }: { onAdd: (body: Parameters<typeof createStory>[3]) => Promise<boolean>; onClose: () => void }) {
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [topic, setTopic] = useState("");
  const [priority, setPriority] = useState("3");
  const [criteria, setCriteria] = useState("");
  const [failed, setFailed] = useState(false);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const added = await onAdd({ title, text, topic, priority: Number(priority), criteria: criteriaOf(criteria) });
    if (added) onClose();
    else setFailed(true);
  };
  const field = "w-full rounded-md border border-input bg-background px-2 py-1.5 text-sm";
  return (
    <form onSubmit={submit} aria-label="Add story" className="max-w-xl space-y-3 rounded-xl border bg-card p-4 text-sm">
      <label className="block space-y-1">
        Title
        <input value={title} onChange={(event) => setTitle(event.target.value)} className={field} />
      </label>
      <label className="block space-y-1">
        Text
        <textarea value={text} onChange={(event) => setText(event.target.value)} rows={3} className={field} />
      </label>
      <label className="block space-y-1">
        Topic
        <input value={topic} onChange={(event) => setTopic(event.target.value)} className={field} />
      </label>
      <label className="block space-y-1">
        Priority
        <select value={priority} onChange={(event) => setPriority(event.target.value)} className={field}>
          {["1", "2", "3"].map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>
      </label>
      <label className="block space-y-1">
        Criteria, one per line
        <textarea value={criteria} onChange={(event) => setCriteria(event.target.value)} rows={4} className={field} />
      </label>
      {failed && (
        <p role="alert" className="text-xs text-destructive">
          The story could not be added. Check the title and try again.
        </p>
      )}
      <div className="flex gap-2">
        <Button type="submit" disabled={title.trim() === ""}>
          Create story
        </Button>
        <Button type="button" variant="ghost" onClick={onClose}>
          Close
        </Button>
      </div>
    </form>
  );
}

/** The board of the feature `featureId`. */
export function Board({
  workspaceId,
  projectId,
  featureId,
}: {
  workspaceId: string;
  projectId: string;
  featureId: string;
}) {
  const load = useCallback(() => loadStories(workspaceId, projectId, featureId), [workspaceId, projectId, featureId]);
  const { value: loaded, failed, reload } = useReloading(load);
  // Answers of our own writes, shown until a load brings a version as new.
  const [answers, setAnswers] = useState<Map<string, StoryBody>>(new Map());
  const stories = useMemo(() => {
    if (!loaded) return loaded;
    const merged = loaded.map((story) => {
      const answer = answers.get(story.id);
      return answer && answer.version > story.version ? answer : story;
    });
    const known = new Set(merged.map((story) => story.id));
    return [...merged, ...[...answers.values()].filter((answer) => !known.has(answer.id) && answer.version === 1)];
  }, [loaded, answers]);
  useProjectEvents(projectId, (event) => {
    if (event.event.startsWith("story.")) reload();
  });
  const name = useProjectName(workspaceId, projectId);
  const [priority, setPriority] = useState("all");
  const [showCancelled, setShowCancelled] = useState(false);
  const [adding, setAdding] = useState(false);

  const [messages, setMessages] = useState<Map<string, string>>(new Map());

  const remember = (story: StoryBody) => setAnswers((current) => new Map(current).set(story.id, story));
  const say = (id: string, message: string | null) =>
    setMessages((current) => {
      const next = new Map(current);
      if (message) next.set(id, message);
      else next.delete(id);
      return next;
    });
  const steer: Steer = {
    stories: stories ?? [],
    messages,
    run: async (story, write) => {
      try {
        remember(await write());
        say(story.id, null);
      } catch (error) {
        if (error instanceof ApiError && error.status === 409) {
          setAnswers(new Map());
          reload();
          say(story.id, CONFLICT);
          return;
        }
        say(story.id, "The change could not be saved. Try again in a moment.");
      }
    },
    patch: (story, body) => patchStory(workspaceId, projectId, featureId, story.id, { expected_version: story.version, ...body }),
    cancel: (story) => cancelStory(workspaceId, projectId, featureId, story.id, story.version),
    check: (story, criterion) => checkCriterion(workspaceId, projectId, featureId, story.id, story.version, criterion),
  };
  const add = async (body: Parameters<typeof createStory>[3]) => {
    try {
      remember(await createStory(workspaceId, projectId, featureId, body));
      reload();
      return true;
    } catch {
      return false;
    }
  };

  const byId = useMemo(() => new Map((stories ?? []).map((story) => [story.id, story])), [stories]);
  const shown = useMemo(
    () => (stories ?? []).filter((story) => priority === "all" || String(story.priority) === priority),
    [stories, priority],
  );
  const columns = showCancelled ? [...COLUMNS, CANCELLED] : COLUMNS;

  return (
    <main className="mx-auto max-w-[96rem] px-5 py-8 md:px-10 md:py-12">
      <PageHeader eyebrow={name ?? "Project"} title="Board" description="The stories of the feature, by where they are." />
      {failed && (
        <Alert variant="destructive" className="mt-6">
          <AlertDescription>The board could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {stories && (
        <div className="mt-6 space-y-4">
          <div className="flex flex-wrap items-center gap-4 text-sm">
            <label className="flex items-center gap-2">
              Priority
              <select
                value={priority}
                onChange={(event) => setPriority(event.target.value)}
                className="h-9 rounded-md border border-input bg-background px-2 text-sm"
              >
                {PRIORITIES.map((value) => (
                  <option key={value} value={value}>
                    {value}
                  </option>
                ))}
              </select>
            </label>
            <label className="flex items-center gap-2">
              <input type="checkbox" checked={showCancelled} onChange={(event) => setShowCancelled(event.target.checked)} />
              Show cancelled
            </label>
            <Button type="button" size="sm" variant="outline" className="ml-auto" onClick={() => setAdding(!adding)}>
              Add story
            </Button>
          </div>
          {adding && <AddStory onAdd={add} onClose={() => setAdding(false)} />}
          <div className="flex gap-4 overflow-x-auto pb-2">
            {columns.map((column) => {
              const cards = shown
                .filter((story) => column.statuses.includes(story.status))
                .sort(
                  (a, b) =>
                    column.statuses.indexOf(a.status) - column.statuses.indexOf(b.status) ||
                    a.priority - b.priority ||
                    a.number - b.number,
                );
              return (
                <section key={column.statuses[0]} aria-label={column.title} className="w-72 shrink-0 space-y-3">
                  <h2 className="flex items-center gap-2 text-sm font-medium">
                    {column.title}
                    <span className="text-xs text-muted-foreground tabular-nums">{cards.length}</span>
                  </h2>
                  {cards.length === 0 ? (
                    <Empty>Nothing here</Empty>
                  ) : (
                    <ul className="space-y-2">
                      {cards.map((story) => (
                        <Card key={story.id} story={story} byId={byId} steer={steer} />
                      ))}
                    </ul>
                  )}
                </section>
              );
            })}
          </div>
        </div>
      )}
    </main>
  );
}
