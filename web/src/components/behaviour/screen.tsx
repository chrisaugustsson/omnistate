import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { loadOpenQuestions, loadTree } from "@/api/behaviour";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { featureNames } from "@/components/project/items";
import { useHome } from "@/lib/use-home";
import { useReloading } from "@/lib/reloading";
import { Details } from "./details";
import { Folders } from "./folders";
import { Graph } from "./graph";
import { ancestorsOf, childrenOf } from "./tree";

// A feature's Behaviour screen: folders on the left, the graph in the middle and
// the details of the chosen node on the right. This owns the screen's state.

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

/** The Behaviour screen of the feature `featureId`. */
export function BehaviourScreen({
  workspaceId,
  projectId,
  featureId,
}: {
  workspaceId: string;
  projectId: string;
  featureId: string;
}) {
  const load = useCallback(() => loadTree(workspaceId, projectId, featureId), [workspaceId, projectId, featureId]);
  const { value: nodes, failed } = useReloading(load);
  // How many questions wait on each node; a question a person asked is a remark and does not wait.
  const loadWaiting = useCallback(
    () => loadOpenQuestions(workspaceId, projectId, featureId).catch(() => []),
    [workspaceId, projectId, featureId],
  );
  const { value: openQuestions } = useReloading(loadWaiting);
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
  const choose = (id: string, from: "folders" | "graph" | "search" | "details" | "keys") => {
    if (!nodes) return;
    setChosen(id);
    const above = ancestorsOf(nodes, id);
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
              <Folders nodes={nodes} open={open} waiting={waiting} onToggle={toggle} chosenId={chosen} onChoose={(id) => choose(id, "folders")} />
            </aside>
            <main aria-label="Tree as a graph" className="min-w-0">
              <Graph nodes={nodes} waiting={waiting} chosenId={chosen} onChoose={(id) => choose(id, "graph")} reveal={reveal} />
            </main>
            <Details node={chosenNode} />
          </>
        )}
      </div>
    </div>
  );
}
