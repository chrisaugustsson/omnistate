import { useCallback, useMemo, useState } from "react";
import { loadTree } from "@/api/behaviour";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useReloading } from "@/lib/reloading";
import { Folders } from "./folders";
import { childrenOf } from "./tree";

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
  // Every node starts closed.
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set());
  const toggle = (id: string) =>
    setOpen((before) => {
      const after = new Set(before);
      if (!after.delete(id)) after.add(id);
      return after;
    });
  const stories = useMemo(() => {
    if (!nodes) return 0;
    const root = (childrenOf(nodes).get(null) ?? [])[0];
    return root ? (childrenOf(nodes).get(root.id) ?? []).length : 0;
  }, [nodes]);

  return (
    <div className="h-screen overflow-x-auto">
      <div className="grid h-full min-w-[1092px] grid-cols-[288px_minmax(400px,1fr)_404px] grid-rows-[56px_minmax(0,1fr)]">
        <header className="col-span-3 flex items-center gap-4 border-b px-4">
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
              <Folders nodes={nodes} open={open} onToggle={toggle} />
            </aside>
            <main aria-label="Tree as a graph" className="min-w-0" />
            <section aria-label="Details" className="border-l" />
          </>
        )}
      </div>
    </div>
  );
}
