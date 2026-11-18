import { useCallback, useEffect, useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";
import { Check, Ellipsis, ListFilter, Plus } from "lucide-react";
import { ApiError } from "@/api/client";
import { cancelStory, checkCriterion, createStory, loadStories, patchStory } from "@/api/stories";
import type { LinkedBody, StoryBody } from "@/api/generated";
import { Button } from "@/components/ui/button";
import { usePopover } from "@/components/shell/popover";
import { StateIcon } from "@/components/shell/state-icon";
import { Dialog, DialogError } from "@/components/shell/dialog";
import { Field, Input, Textarea } from "@/components/shell/field";
import { Select } from "@/components/shell/select";
import { MenuSeparator, buttonClass, menuClass, menuItemClass } from "@/components/track/ui";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useProjectEvents } from "@/live/context";
import { useReloading } from "@/lib/reloading";
import { StoryDrawer } from "./story-drawer";
import { cn } from "@/lib/utils";

// A track's board: the five columns of a story's life, a card per story, and
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
const PRIORITIES = [
  { value: "", label: "Any priority" },
  { value: "1", label: "P1" },
  { value: "2", label: "P2" },
  { value: "3", label: "P3" },
];

const closed = (story: StoryBody) => story.status === "done" || story.status === "cancelled";
const unresolved = (story: StoryBody) => story.findings.filter((finding) => finding.resolved !== true).length;
/** Whether a blocker of the story sits on a dropped track and is still open: the story stays blocked. */
const droppedBlocker = (story: StoryBody, byId: Map<string, StoryBody>) =>
  (story.blockers ?? []).some((blocker) => blocker.blocker_dropped && blocker.status !== "done" && blocker.status !== "cancelled" && !byId.has(blocker.id));
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

export type Steer = {
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

/** A card's quiet actions: small, muted, filled on hover. */
const CARD_ACTION = "h-6 rounded-control px-1.5 text-meta font-medium text-ink-2 hover:bg-hover hover:text-ink";

/**
 * A story's card: its key, priority as a small badge, title, holder, flags and
 * what blocks it ("Blocked by iot-service US-3"). Its actions (priority, Edit
 * blockers, Cancel story) are in the "…" menu, which shows on hover or focus;
 * a closed story has none.
 */
function Card({
  story,
  byId,
  linked,
  steer,
  onOpen,
}: {
  story: StoryBody;
  byId: Map<string, StoryBody>;
  linked: LinkedBody[];
  steer: Steer;
  onOpen: () => void;
}) {
  const holder = story.holder as { run?: string; stage?: string } | null | undefined;
  const findings = unresolved(story);
  const check = toCheck(story);
  const isClosed = closed(story);
  const message = steer.messages.get(story.id);
  const [editing, setEditing] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const write = async (action: () => Promise<StoryBody>) => {
    await steer.run(story, action);
  };
  const others = steer.stories.filter((other) => other.id !== story.id);
  const menu = usePopover();
  // The board's columns scroll sideways, which would clip the menu: it is placed on the page, under its button, and closes on a scroll.
  const [at, setAt] = useState<{ top: number; right: number } | null>(null);
  const openMenu = () => {
    const box = menu.opener.current?.getBoundingClientRect();
    if (box) setAt({ top: box.bottom + 4, right: window.innerWidth - box.right });
    menu.toggle();
  };
  useEffect(() => {
    if (!menu.open) return;
    const close = () => menu.setOpen(false);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [menu.open, menu.setOpen]);
  return (
    <li className="group/card relative space-y-2 rounded-control border border-line bg-sheet px-3 py-2.5 shadow-card transition-[border-color,box-shadow] duration-(--t-fast) ease-out-quint hover:border-line-strong has-[>p>button:focus-visible]:shadow-focus">
      <div className="flex h-6 min-w-0 items-center gap-2 text-meta text-ink-3">
        <span className="font-mono text-[12px] tabular-nums">{story.key}</span>
        {(story.status === "implemented" || story.status === "reviewed") && <Badge tone="neutral">Waiting</Badge>}
        <span
          title={story.priority === 0 ? "Priority 0: a fix" : `Priority ${story.priority}`}
          className="ml-auto inline-flex h-5 items-center rounded-badge bg-hover px-1.5 text-meta font-medium text-ink-2 tabular-nums"
        >
          P{story.priority}
        </span>
        {!isClosed && (
          <div ref={menu.anchor} className="relative z-10 -mr-1.5 inline-flex">
            <button
              ref={menu.opener}
              type="button"
              aria-label={`${story.key} actions`}
              aria-haspopup="menu"
              aria-expanded={menu.open}
              onClick={openMenu}
              className={cn(
                CARD_ACTION,
                "grid w-6 place-items-center px-0 opacity-0 group-focus-within/card:opacity-100 group-hover/card:opacity-100 focus-visible:opacity-100 aria-expanded:opacity-100",
              )}
            >
              <Ellipsis aria-hidden strokeWidth={2.25} className="size-(--icon-sm)" />
            </button>
            {menu.open && (
              <div
                role="menu"
                aria-label={`${story.key} actions`}
                style={at ? { top: at.top, right: at.right } : undefined}
                className={cn(menuClass, "fixed min-w-[180px]", !at && "absolute")}
              >
                {story.priority !== 0 && (
                  <>
                    <div className="flex h-7 items-center px-2 text-meta text-ink-3">Priority</div>
                    {[1, 2, 3].map((value) => (
                      <button
                        key={value}
                        type="button"
                        role="menuitemradio"
                        aria-checked={story.priority === value}
                        onClick={() => {
                          menu.setOpen(false);
                          if (story.priority !== value) void write(() => steer.patch(story, { priority: value }));
                        }}
                        className={menuItemClass}
                      >
                        <span className="grid w-4 place-items-center">{story.priority === value && <Check aria-hidden className="size-(--icon-sm)" />}</span>
                        {`P${value}`}
                      </button>
                    ))}
                    <MenuSeparator />
                  </>
                )}
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    menu.setOpen(false);
                    setEditing(true);
                  }}
                  className={menuItemClass}
                >
                  Edit blockers…
                </button>
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => {
                    menu.setOpen(false);
                    setConfirming(true);
                  }}
                  className={cn(menuItemClass, "text-danger-ink")}
                >
                  Cancel story…
                </button>
              </div>
            )}
          </div>
        )}
      </div>
      <p className="font-medium text-ink">
        {/* The title opens the story; its hit area covers the card, under the card's own controls. */}
        <button type="button" onClick={onOpen} className="text-left outline-none after:absolute after:inset-0 after:rounded-control after:content-['']">
          {story.title}
        </button>
      </p>
      {holder && (
        <p className="text-xs text-muted-foreground">
          {holder.stage} · {holder.run}
        </p>
      )}
      {(story.blocked || droppedBlocker(story, byId) || findings > 0 || check > 0) && (
        <div className="relative z-10 flex w-fit flex-wrap items-center gap-1.5">
          {story.blocked && <Badge tone="warning">Blocked</Badge>}
          {droppedBlocker(story, byId) && <Badge tone="warning">Blocker dropped</Badge>}
          {findings > 0 && <Badge tone="neutral">{`${findings} findings`}</Badge>}
          {check > 0 && (
            <button type="button" onClick={onOpen} className="rounded-full focus-visible:shadow-focus focus-visible:outline-none">
              <Badge tone="accent">{`${check} to check`}</Badge>
            </button>
          )}
        </div>
      )}
      {story.blocked_by.length > 0 && (
        <ul aria-label="Blocked by" className="flex flex-col gap-0.5 text-meta text-ink-3">
          {story.blocked_by.map((id) => {
            const blocker = byId.get(id) ?? story.blockers?.find((other) => other.id === id);
            // A blocker on another track is named with its part, as the track's panel names it.
            const part = byId.has(id) ? undefined : linked.find((task) => task.id === id)?.part_name;
            const finished = blocker && (blocker.status === "done" || blocker.status === "cancelled");
            return (
              <li key={id} className="min-w-0 break-words">
                Blocked by{" "}
                <span className="whitespace-nowrap">
                  {part && <>{part} </>}
                  <span className={cn("inline-flex items-center gap-0.5 align-bottom font-mono text-[12px] text-ink-2", finished && "line-through decoration-ink-3")}>
                    {blocker?.key ?? "?"}
                    {finished && <Check aria-label="done" className="size-3 text-state-done" />}
                  </span>
                </span>
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
      {editing && !isClosed && (
        <div className="relative z-10 space-y-1 rounded-control bg-frame px-2 py-1.5 text-xs">
          <div className="flex items-center text-meta text-ink-3">
            Blocked by
            <button type="button" onClick={() => setEditing(false)} className={cn(CARD_ACTION, "ml-auto -mr-1")}>
              Done
            </button>
          </div>
          <ul className="space-y-1">
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
        </div>
      )}
      {confirming && !isClosed && (
        <div className="relative z-10 flex flex-wrap items-center gap-2 text-xs">
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

const PRIORITY_OPTIONS = [
  { value: "1", label: "P1", description: "Delivered first" },
  { value: "2", label: "P2" },
  { value: "3", label: "P3", description: "The default" },
];

/** The dialog that adds a story to the backlog: its title (required), text, topic, priority and criteria, one per line. */
function AddStory({ onAdd, onClose }: { onAdd: (body: Parameters<typeof createStory>[3]) => Promise<boolean>; onClose: () => void }) {
  const [title, setTitle] = useState("");
  const [text, setText] = useState("");
  const [topic, setTopic] = useState("");
  const [priority, setPriority] = useState("3");
  const [criteria, setCriteria] = useState("");
  const [tried, setTried] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const submit = async () => {
    setTried(true);
    if (title.trim() === "") {
      requestAnimationFrame(() => document.querySelector<HTMLElement>('[role="dialog"] [aria-invalid="true"]')?.focus());
      return;
    }
    setBusy(true);
    setFailed(false);
    const added = await onAdd({ title, text, topic, priority: Number(priority), criteria: criteriaOf(criteria) });
    setBusy(false);
    if (added) onClose();
    else setFailed(true);
  };
  return (
    <Dialog
      wide
      title="Add a story"
      description="It goes into the backlog; the deliver agents pick it up by priority once the track is building."
      onClose={onClose}
      onSubmit={() => {
        if (!busy) void submit();
      }}
      footerStart={failed && <DialogError>The story could not be added. Check the title and try again.</DialogError>}
      footer={
        <>
          <button type="button" className={buttonClass()} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className={buttonClass({ variant: "primary" })} disabled={busy}>
            Add story
          </button>
        </>
      }
    >
      <Field label="Title" error={tried && title.trim() === "" ? "Give the story a title" : undefined}>
        {(control) => <Input {...control} placeholder="What a person can do once it is built" value={title} onChange={(event) => setTitle(event.target.value)} />}
      </Field>
      <Field label="Text" optional>
        {(control) => <Textarea {...control} minRows={3} placeholder="As a …, I want …, so that …" value={text} onChange={(event) => setText(event.target.value)} />}
      </Field>
      <div className="mt-4 grid grid-cols-[minmax(0,1fr)_140px] gap-3 [&>*]:mt-0!">
        <Field label="Topic" optional>
          {(control) => <Input {...control} placeholder="checkout" value={topic} onChange={(event) => setTopic(event.target.value)} />}
        </Field>
        <Field label="Priority">{(control) => <Select {...control} value={priority} onChange={setPriority} options={PRIORITY_OPTIONS} />}</Field>
      </div>
      <Field label="Criteria" optional hint={'One per line. A line with "(human check)" waits on a person ticking it.'} className="mt-4">
        {(control) => (
          <Textarea {...control} minRows={4} placeholder={"The total is shown.\nThe receipt reads well (human check)"} value={criteria} onChange={(event) => setCriteria(event.target.value)} />
        )}
      </Field>
    </Dialog>
  );
}

/** The stories of other tracks that block or are blocked by this track's stories: read-only, each opening its own track's board. */
function LinkedTasks({ workspaceId, projectId, linked }: { workspaceId: string; projectId: string; linked: LinkedBody[] }) {
  return (
    <section aria-label="Linked tasks" className="pt-6">
      <h2 className="mb-2 flex h-8 items-center gap-2 text-section font-semibold">
        Linked tasks on other parts <span className="font-medium text-ink-3 tabular-nums">{linked.length}</span>
      </h2>
      <ul className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
        {linked.map((task) => (
          <li key={`${task.id}-${task.relation}-${task.story_id}`}>
            <Link
              to="/w/$workspaceId/projects/$projectId/tracks/$trackId/board"
              params={{ workspaceId, projectId, trackId: task.track_id }}
              className="block space-y-1 rounded-control border border-line bg-sheet px-3 py-2.5 transition-colors hover:bg-frame"
            >
              <span className="flex items-center gap-2 text-xs text-muted-foreground">
                <span className="font-medium tabular-nums">{task.key}</span>
                <span>{task.relation === "blocks" ? `blocks ${task.story_key}` : `blocked by ${task.story_key}`}</span>
                <span className="ml-auto">{task.status}</span>
              </span>
              <span className="block font-medium">{task.title}</span>
              <span className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
                {`${task.part_name} · ${task.track_name}`}
                {task.blocker_dropped && <Badge tone="warning">Dropped</Badge>}
              </span>
            </Link>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** The board of the track `trackId`. */
export function Board({
  workspaceId,
  projectId,
  trackId,
}: {
  workspaceId: string;
  projectId: string;
  trackId: string;
}) {
  const load = useCallback(() => loadStories(workspaceId, projectId, trackId), [workspaceId, projectId, trackId]);
  const { value: board, failed, reload } = useReloading(load);
  const loaded = board?.stories;
  const linked = board?.linked ?? [];
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
  const [priority, setPriority] = useState("");
  const [showCancelled, setShowCancelled] = useState(false);
  const [adding, setAdding] = useState(false);
  const [openId, setOpenId] = useState<string | null>(null);

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
    patch: (story, body) => patchStory(workspaceId, projectId, trackId, story.id, { expected_version: story.version, ...body }),
    cancel: (story) => cancelStory(workspaceId, projectId, trackId, story.id, story.version),
    check: (story, criterion) => checkCriterion(workspaceId, projectId, trackId, story.id, story.version, criterion),
  };
  const add = async (body: Parameters<typeof createStory>[3]) => {
    try {
      remember(await createStory(workspaceId, projectId, trackId, body));
      reload();
      return true;
    } catch {
      return false;
    }
  };

  const byId = useMemo(() => new Map((stories ?? []).map((story) => [story.id, story])), [stories]);
  const shown = useMemo(
    () => (stories ?? []).filter((story) => priority === "" || String(story.priority) === priority),
    [stories, priority],
  );
  const columns = showCancelled ? [...COLUMNS, CANCELLED] : COLUMNS;
  const opened = openId ? byId.get(openId) : undefined;

  return (
    <div>
      {failed && (
        <Alert variant="destructive">
          <AlertDescription>The board could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {stories && (
        <div className="space-y-4">
          <div className="flex min-h-8 flex-wrap items-center gap-4">
            <p className="text-ink-3">
              <span className="font-medium text-ink">
                {stories.filter((story) => story.status === "done").length} of {stories.filter((story) => story.status !== "cancelled").length}
              </span>{" "}
              stories done
            </p>
            <div role="group" aria-label="Filter" className="ml-auto flex items-center gap-1">
              <ListFilter aria-hidden className="mr-1 size-(--icon-sm) text-ink-3" />
              <Select look="filter" aria-label="Priority" value={priority} onChange={setPriority} options={PRIORITIES} />
              <label className="inline-flex h-(--control-h) cursor-pointer items-center gap-1.5 rounded-control px-2 font-medium text-ink-2 transition-colors duration-(--t-fast) hover:bg-hover has-checked:bg-selected has-checked:text-ink has-focus-visible:shadow-focus">
                <input type="checkbox" className="size-3.5 accent-(--brand)" checked={showCancelled} onChange={(event) => setShowCancelled(event.target.checked)} />
                Show cancelled
              </label>
            </div>
            <button type="button" className={buttonClass()} onClick={() => setAdding(true)}>
              <Plus aria-hidden strokeWidth={2.25} />
              Add story
            </button>
          </div>
          {adding && <AddStory onAdd={add} onClose={() => setAdding(false)} />}
          {opened && <StoryDrawer story={opened} byId={byId} linked={linked} steer={steer} onClose={() => setOpenId(null)} />}
          {/* The columns share the width down to a 1024px window; only below that does the board scroll sideways. */}
          <div className="overflow-x-auto max-[1023px]:pb-2">
            <div
              className="grid items-start gap-2 max-[1023px]:min-w-[880px]"
              style={{ gridTemplateColumns: `repeat(${columns.length}, minmax(0, 1fr))` }}
            >
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
                <section key={column.statuses[0]} aria-label={column.title} className="min-w-0 rounded-card bg-well p-1.5 pb-2">
                  <h2 className="flex h-8 items-center gap-2 px-1.5 font-medium text-ink">
                    <StateIcon state={column.statuses[column.statuses.length - 1]} />
                    {column.title}
                    <span className="font-medium text-ink-3 tabular-nums">{cards.length}</span>
                  </h2>
                  {cards.length === 0 ? (
                    <p className="px-1.5 pt-1 text-meta text-ink-3">None yet</p>
                  ) : (
                    <ul className="space-y-1.5">
                      {cards.map((story) => (
                        <Card key={story.id} story={story} byId={byId} linked={linked} steer={steer} onOpen={() => setOpenId(story.id)} />
                      ))}
                    </ul>
                  )}
                </section>
              );
            })}
            </div>
          </div>
          {linked.length > 0 && <LinkedTasks workspaceId={workspaceId} projectId={projectId} linked={linked} />}
        </div>
      )}
    </div>
  );
}
