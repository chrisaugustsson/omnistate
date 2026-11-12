import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { loadOpenQuestions, loadTree } from "@/api/behaviour";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { featureNames } from "@/components/project/items";
import { useHome } from "@/lib/use-home";
import { useReloading } from "@/lib/reloading";
import { Details } from "./details";
import { Folders } from "./folders";
import { Graph } from "./graph";
import { ancestorsOf, childrenOf, matches, shownUnder, type Filter } from "./tree";

// A feature's Behaviour screen: folders on the left, the graph in the middle and
// the details of the chosen node on the right. This owns the screen's state.

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

/** The Behaviour screen of the feature `featureId`. */
export function BehaviourScreen({
  workspaceId,
  projectId,
  featureId,
  me = null,
}: {
  workspaceId: string;
  projectId: string;
  featureId: string;
  me?: string | null;
}) {
  const load = useCallback(() => loadTree(workspaceId, projectId, featureId), [workspaceId, projectId, featureId]);
  const { value: nodes, failed, reload: reloadTree } = useReloading(load);
  // How many questions wait on each node; a question a person asked is a remark and does not wait.
  const loadWaiting = useCallback(
    () => loadOpenQuestions(workspaceId, projectId, featureId).catch(() => []),
    [workspaceId, projectId, featureId],
  );
  const { value: openQuestions, reload: reloadQuestions } = useReloading(loadWaiting);
  const waiting = useMemo(() => {
    const counts = new Map<string, number>();
    for (const question of openQuestions ?? []) {
      if (question.asked_by.kind !== "human") counts.set(question.node_id, (counts.get(question.node_id) ?? 0) + 1);
    }
    return counts;
  }, [openQuestions]);
  const { home } = useHome(workspaceId, projectId);
  const featureName = home ? (featureNames(home).get(featureId) ?? "") : "";
  // Every node starts closed.
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set());
  const toggle = (id: string) =>
    setOpen((before) => {
      const after = new Set(before);
      if (!after.delete(id)) after.add(id);
      return after;
    });
  // One chosen node for all three parts; a choice made outside the graph also brings it into sight.
  const [chosen, setChosen] = useState<string | null>(null);
  const [reveal, setReveal] = useState<{ id: string; ms: number; n: number } | null>(null);
  const revealCount = useRef(0);
  const [filter, setFilterState] = useState<Filter>("all");
  const [fitN, setFitN] = useState(0);
  const [fitMs, setFitMs] = useState(0);
  const setFilter = (next: Filter) => {
    setFilterState(next);
    setFitMs(0);
    setFitN((n) => n + 1);
  };
  // Folds belong to the graph; the folders keep their own open and closed rows.
  const [folded, setFolded] = useState<ReadonlySet<string>>(new Set());
  const foldOne = (id: string) =>
    setFolded((before) => {
      const after = new Set(before);
      if (!after.delete(id)) after.add(id);
      return after;
    });
  const choose = (id: string, from: "folders" | "graph" | "search" | "details" | "keys") => {
    if (!nodes) return;
    setChosen(id);
    const above = ancestorsOf(nodes, id);
    // Pressing a closed row with children also opens it.
    if (from === "folders" && filter === "all" && nodes.some((node) => node.parent_id === id)) above.push(id);
    // A fold that hides the node is undone.
    setFolded((before) => (above.some((one) => before.has(one)) ? new Set([...before].filter((one) => !above.includes(one))) : before));
    setOpen((before) => (above.every((one) => before.has(one)) ? before : new Set([...before, ...above])));
    if (from !== "graph") setReveal({ id, ms: from === "keys" ? 0 : 240, n: ++revealCount.current });
  };
  const chooseRef = useRef(choose);
  chooseRef.current = choose;
  // The arrow keys choose, unless a field has focus or ⌘, Ctrl or Alt is held.
  useEffect(() => {
    if (!nodes) return;
    const onKey = (event: KeyboardEvent) => {
      if (!event.key.startsWith("Arrow") || event.metaKey || event.ctrlKey || event.altKey) return;
      const target = event.target as HTMLElement | null;
      if (target && (["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName) || target.isContentEditable)) return;
      const children = childrenOf(nodes);
      const root = (children.get(null) ?? [])[0];
      if (!root) return;
      const node = chosen ? nodes.find((one) => one.id === chosen) : undefined;
      let to: string | undefined;
      if (!node) to = (children.get(root.id) ?? [])[0]?.id;
      else {
        const brothers = children.get(node.parent_id ?? null) ?? [];
        const at = brothers.findIndex((one) => one.id === node.id);
        if (event.key === "ArrowLeft") to = node.parent_id && node.parent_id !== root.id ? node.parent_id : undefined;
        else if (event.key === "ArrowRight") to = (children.get(node.id) ?? [])[0]?.id;
        else if (event.key === "ArrowUp") to = node.parent_id ? brothers[at - 1]?.id : undefined;
        else if (event.key === "ArrowDown") to = node.parent_id ? brothers[at + 1]?.id : undefined;
      }
      if (!to) return;
      event.preventDefault();
      chooseRef.current(to, "keys");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [nodes, chosen]);
  const chosenNode = useMemo(() => nodes?.find((node) => node.id === chosen) ?? null, [nodes, chosen]);
  const under = useMemo(() => (nodes && filter !== "all" ? shownUnder(nodes, filter, chosen) : null), [nodes, filter, chosen]);
  const counts = useMemo(() => {
    const all = nodes ?? [];
    const count = (one: Filter) => all.filter((node) => matches(node, one)).length;
    return { all: count("all"), open: count("open"), default: count("default"), needs_pass: count("needs_pass"), answers: all.reduce((sum, node) => sum + node.default_answers, 0) };
  }, [nodes]);
  const waitingTotal = [...waiting.values()].reduce((sum, count) => sum + count, 0);
  const choices = ([["all", "All"], ["open", "Only open"], ["default", "Only by default"], ["needs_pass", "Only needing a grill pass"]] as const).filter(
    ([name]) => name !== "needs_pass" || counts.needs_pass > 0 || filter === "needs_pass",
  );
  // The stories with something under them that the filter shows.
  const foldable = useMemo(() => {
    if (!nodes) return [];
    const kids = childrenOf(nodes);
    const root = (kids.get(null) ?? [])[0];
    const shown = under?.shown;
    return (root ? (kids.get(root.id) ?? []) : []).filter((story) => (kids.get(story.id) ?? []).some((child) => !shown || shown.has(child.id)));
  }, [nodes, under]);
  const allFolded = foldable.length > 0 && foldable.every((story) => folded.has(story.id));
  const foldAll = () => {
    setFolded(allFolded ? new Set() : new Set(foldable.map((story) => story.id)));
    setFitMs(260);
    setFitN((n) => n + 1);
  };
  const stories = useMemo(() => {
    if (!nodes) return 0;
    const root = (childrenOf(nodes).get(null) ?? [])[0];
    return root ? (childrenOf(nodes).get(root.id) ?? []).length : 0;
  }, [nodes]);

  return (
    <div className="h-full overflow-x-auto">
      <div className="grid h-full min-w-[1092px] grid-cols-[288px_minmax(400px,1fr)_404px] grid-rows-[56px_minmax(0,1fr)]">
        <header className="col-span-3 flex items-center gap-4 border-b px-4">
          <span
            aria-hidden
            className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-primary font-display text-lg leading-none text-primary-foreground"
          >
            O
          </span>
          <Link
            to="/w/$workspaceId/projects/$projectId/features/$featureId"
            params={{ workspaceId, projectId, featureId }}
            className="max-w-64 truncate text-[13px] font-medium text-muted-foreground hover:text-foreground"
          >
            ‹ {featureName}
          </Link>
          <h1 className="font-display text-2xl leading-none tracking-[-0.01em]">Behaviour</h1>
          {nodes && nodes.length > 0 && (
            <>
              <div role="group" aria-label="Show" className="ml-2 flex shrink-0 rounded-lg bg-sidebar-accent p-0.5">
                {choices.map(([name, label]) => (
                  <button
                    key={name}
                    type="button"
                    aria-pressed={filter === name}
                    onClick={() => setFilter(filter === name && name !== "all" ? "all" : name)}
                    className={cn(
                      "flex h-7 items-center gap-1.5 whitespace-nowrap rounded-md px-2.5 text-[12.5px] text-sidebar-foreground",
                      filter === name && "bg-background font-medium text-foreground shadow-[0_0_0_1px_var(--border)]",
                    )}
                  >
                    {label}
                    <span className="text-[11.5px] font-normal tabular-nums text-muted-foreground">{counts[name]}</span>
                  </button>
                ))}
              </div>
              <p data-facts className="min-w-0 truncate text-[12.5px] text-muted-foreground">
                {waitingTotal > 0 ? (
                  <button type="button" onClick={() => setFilter("open")} className="text-foreground underline-offset-2 hover:underline">
                    {plural(waitingTotal, "question waits", "questions wait")} for an answer
                  </button>
                ) : (
                  "No question is open"
                )}
                {counts.answers > 0 && ` · ${counts.answers} answers were given by default`}
                {counts.needs_pass > 0 && ` · ${plural(counts.needs_pass, "node waits", "nodes wait")} for the next grill pass`}
              </p>
              <Button type="button" variant="outline" size="sm" onClick={foldAll} className="ml-auto shrink-0">
                {allFolded ? "Unfold all stories" : "Fold all stories"}
              </Button>
            </>
          )}
        </header>
        {failed && (
          <Alert variant="destructive" className="col-span-3 m-4 w-auto">
            <AlertDescription>The behaviour tree could not be loaded. Try again in a moment.</AlertDescription>
          </Alert>
        )}
        {nodes && nodes.length === 0 && (
          <p className="col-span-3 p-4 text-sm text-muted-foreground">No behaviour has been written for this feature yet.</p>
        )}
        {nodes && nodes.length > 0 && (
          <>
            <aside aria-label="Tree as folders" className="flex min-h-0 flex-col border-r">
              <p className="px-4 pb-1.5 pt-3 text-[11.5px] text-muted-foreground">
                {plural(stories, "story", "stories")} · {plural(nodes.length, "node", "nodes")}
              </p>
              <Folders nodes={nodes} shown={under?.shown ?? null} matching={under?.matching ?? null} open={open} waiting={waiting} onToggle={toggle} chosenId={chosen} onChoose={(id) => choose(id, "folders")} />
            </aside>
            <main aria-label="Tree as a graph" className="relative min-w-0">
              <Graph
                nodes={nodes}
                waiting={waiting}
                chosenId={chosen}
                onChoose={(id) => choose(id, "graph")}
                reveal={reveal}
                shown={under?.shown ?? null}
                matching={under?.matching ?? null}
                fitN={fitN}
                fitMs={fitMs}
                folded={folded}
                onFold={foldOne}
              />
              {under && (
                <p data-no-pan className="absolute right-3 top-3 flex items-center gap-2 rounded-lg border bg-background px-2.5 py-1.5 text-[11.5px] text-muted-foreground">
                  <span>
                    {under.matching.size} of {nodes.length} nodes{under.matching.size > 0 && ", with what lies above them"}
                  </span>
                  <button type="button" onClick={() => setFilter("all")} className="font-medium text-foreground hover:underline">
                    Show all
                  </button>
                </p>
              )}
            </main>
            <Details
              node={chosenNode}
              tree={nodes}
              me={me}
              onChoose={(id) => choose(id, "details")}
              workspaceId={workspaceId}
              projectId={projectId}
              featureId={featureId}
              onWritten={() => {
                reloadTree();
                reloadQuestions();
              }}
              waitingCount={waitingTotal}
              onShowOpen={() => setFilter("open")}
            />
          </>
        )}
      </div>
    </div>
  );
}
