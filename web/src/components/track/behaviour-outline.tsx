import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { loadOpenQuestions, loadTree } from "@/api/behaviour";
import { Details } from "@/components/behaviour/details";
import { Folders } from "@/components/behaviour/folders";
import { ancestorsOf, childrenOf, matches, shownUnder } from "@/components/behaviour/tree";
import { useReloading } from "@/lib/reloading";
import { cn } from "@/lib/utils";
import { useProjectEvents } from "@/live/context";
import { EmptyTab } from "./ui";

// A track's Behaviour on the phone: the tree as an indented outline (the
// Behaviour screen's folders), its stories open, with how many questions wait
// and a switch to only the open nodes. Tapping a node opens its details full
// screen, where its questions are answered; × closes them. The graph is left
// out: it does not fit a phone.

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

export function BehaviourOutline({
  workspaceId,
  projectId,
  trackId,
  me,
  onWaiting,
}: {
  workspaceId: string;
  projectId: string;
  trackId: string;
  me: string | null;
  onWaiting?: (count: number) => void;
}) {
  const load = useCallback(() => loadTree(workspaceId, projectId, trackId), [workspaceId, projectId, trackId]);
  const { value: nodes, failed, reload: reloadTree } = useReloading(load, `tree:${trackId}`);
  const loadWaiting = useCallback(() => loadOpenQuestions(workspaceId, projectId, trackId).catch(() => []), [workspaceId, projectId, trackId]);
  const { value: questions, reload: reloadQuestions } = useReloading(loadWaiting, `questions:${trackId}`);
  const [tick, setTick] = useState(0);
  const [chosen, setChosen] = useState<string | null>(null);
  const [onlyOpen, setOnlyOpen] = useState(false);
  const [open, setOpen] = useState<ReadonlySet<string> | null>(null);

  // A question a person asked is a remark and does not wait.
  const waiting = useMemo(() => {
    const counts = new Map<string, number>();
    for (const question of questions ?? []) {
      if (question.asked_by.kind !== "human") counts.set(question.node_id, (counts.get(question.node_id) ?? 0) + 1);
    }
    return counts;
  }, [questions]);
  const total = [...waiting.values()].reduce((sum, count) => sum + count, 0);
  useEffect(() => {
    if (questions) onWaiting?.(total);
  }, [questions, total, onWaiting]);

  // The stories start open, so the outline shows the situations under them.
  const stories = useMemo(() => {
    if (!nodes) return [];
    const kids = childrenOf(nodes);
    const root = (kids.get(null) ?? [])[0];
    return root ? (kids.get(root.id) ?? []) : [];
  }, [nodes]);
  const shownOpen = open ?? new Set(stories.map((story) => story.id));
  const toggle = (id: string) =>
    setOpen(() => {
      const after = new Set(shownOpen);
      if (!after.delete(id)) after.add(id);
      return after;
    });

  const chosenRef = useRef(chosen);
  chosenRef.current = chosen;
  useProjectEvents(projectId, (event) => {
    if (!event.event.startsWith("behaviour.") || event.data.track_id !== trackId) return;
    reloadTree();
    reloadQuestions();
    if (chosenRef.current !== null && event.data.node_id === chosenRef.current) setTick((n) => n + 1);
  });

  const under = useMemo(() => (nodes && onlyOpen ? shownUnder(nodes, "open", chosen) : null), [nodes, onlyOpen, chosen]);
  const openCount = useMemo(() => (nodes ?? []).filter((node) => matches(node, "open")).length, [nodes]);
  const chosenNode = nodes?.find((node) => node.id === chosen) ?? null;
  const choose = (id: string) => {
    if (!nodes) return;
    const above = ancestorsOf(nodes, id);
    setOpen(new Set([...shownOpen, ...above]));
    setChosen(id);
  };

  if (failed) return <p className="text-danger-ink">The behaviour tree could not be loaded. Try again in a moment.</p>;
  if (!nodes) return <div aria-hidden className="h-40 animate-pulse rounded-card bg-frame" />;
  if (nodes.length === 0) {
    return (
      <EmptyTab title="No behaviour yet">
        The behaviour agent grills the track&rsquo;s user stories and every situation under them into a tree here, and leaves its open questions for you to answer.
      </EmptyTab>
    );
  }

  return (
    <section aria-label="Behaviour outline" className="-mx-2">
      <div className="mb-2 flex flex-wrap items-center gap-x-3 gap-y-2 px-2">
        <p className="min-w-0 flex-1 text-ink-2">
          {total > 0 ? <span className="font-medium text-ink">{plural(total, "question waits", "questions wait")}</span> : "No question is open"}
        </p>
        <div role="group" aria-label="Show" className="flex shrink-0 rounded-control bg-frame p-0.5">
          {(
            [
              [false, "All", nodes.length],
              [true, "Open", openCount],
            ] as const
          ).map(([value, label, count]) => (
            <button
              key={label}
              type="button"
              aria-pressed={onlyOpen === value}
              onClick={() => setOnlyOpen(value)}
              className={cn(
                "flex h-7 items-center gap-1.5 rounded-[5px] px-2.5 text-meta font-medium text-ink-2",
                onlyOpen === value && "bg-sheet text-ink shadow-control",
              )}
            >
              {label}
              <span className="font-normal text-ink-3 tabular-nums">{count}</span>
            </button>
          ))}
        </div>
      </div>
      <div className="flex flex-col [&>div]:pb-2 [&_[data-row]]:h-9 [&_[data-row]]:text-body">
        <Folders
          nodes={nodes}
          open={shownOpen}
          waiting={waiting}
          onToggle={toggle}
          chosenId={chosen}
          onChoose={choose}
          shown={under?.shown ?? null}
          matching={under?.matching ?? null}
        />
      </div>
      {chosenNode && (
        <NodeScreen title={chosenNode.title} onClose={() => setChosen(null)}>
          <Details
            node={chosenNode}
            tree={nodes}
            me={me}
            onChoose={choose}
            onClose={() => setChosen(null)}
            workspaceId={workspaceId}
            projectId={projectId}
            trackId={trackId}
            onWritten={() => {
              reloadTree();
              reloadQuestions();
            }}
            refresh={tick}
          />
        </NodeScreen>
      )}
    </section>
  );
}

/** A node's details over the whole screen; Esc closes it, and the focus goes back where it was. */
function NodeScreen({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    box.current?.focus({ preventScroll: true });
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") close.current();
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      if (before && document.contains(before)) before.focus({ preventScroll: true });
    };
  }, []);
  return (
    <div
      ref={box}
      role="dialog"
      aria-modal="true"
      aria-label={title}
      tabIndex={-1}
      className="fixed inset-0 z-50 flex flex-col bg-sheet pb-[env(safe-area-inset-bottom)] outline-none [&>section]:flex-1 [&>section]:border-l-0"
    >
      {children}
    </div>
  );
}
