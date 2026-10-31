import { useCallback, useMemo, useState } from "react";
import { Check } from "lucide-react";
import { loadStories } from "@/api/stories";
import type { StoryBody } from "@/api/generated";
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

const COLUMNS = [
  { status: "backlog", title: "Backlog" },
  { status: "implementing", title: "Implementing" },
  { status: "review", title: "Review" },
  { status: "testing", title: "Testing" },
  { status: "done", title: "Done" },
];
const CANCELLED = { status: "cancelled", title: "Cancelled" };
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

function Card({ story, byId }: { story: StoryBody; byId: Map<string, StoryBody> }) {
  const holder = story.holder as { run?: string; stage?: string } | null | undefined;
  const findings = unresolved(story);
  const check = toCheck(story);
  return (
    <li className="space-y-2 rounded-xl border bg-card px-3 py-2.5 text-sm">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span className="font-medium tabular-nums">{story.key}</span>
        <span className="ml-auto tabular-nums">{`P${story.priority}`}</span>
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
        {check > 0 && <Badge tone="accent">{`${check} to check`}</Badge>}
      </div>
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
    </li>
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
  const { value: stories, failed, reload } = useReloading(load);
  useProjectEvents(projectId, (event) => {
    if (event.event.startsWith("story.")) reload();
  });
  const name = useProjectName(workspaceId, projectId);
  const [priority, setPriority] = useState("all");
  const [showCancelled, setShowCancelled] = useState(false);

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
          </div>
          <div className="flex gap-4 overflow-x-auto pb-2">
            {columns.map((column) => {
              const cards = shown
                .filter((story) => story.status === column.status)
                .sort((a, b) => a.priority - b.priority || a.number - b.number);
              return (
                <section key={column.status} aria-label={column.title} className="w-72 shrink-0 space-y-3">
                  <h2 className="flex items-center gap-2 text-sm font-medium">
                    {column.title}
                    <span className="text-xs text-muted-foreground tabular-nums">{cards.length}</span>
                  </h2>
                  {cards.length === 0 ? (
                    <Empty>Nothing here</Empty>
                  ) : (
                    <ul className="space-y-2">
                      {cards.map((story) => (
                        <Card key={story.id} story={story} byId={byId} />
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
