import { useEffect, useRef } from "react";
import { ChevronRight } from "lucide-react";
import type { TreeNode } from "@/api/behaviour";
import { Marks } from "./marks";
import { childrenOf, shortTitle } from "./tree";
import { cn } from "@/lib/utils";

/**
 * The tree as folders: the feature first, always open, then its stories and
 * what lies under them. `open` holds the ids of the nodes whose children show.
 */
export function Folders({
  nodes,
  open,
  waiting,
  onToggle,
  chosenId = null,
  onChoose,
  shown = null,
  matching = null,
}: {
  nodes: TreeNode[];
  open: ReadonlySet<string>;
  waiting: ReadonlyMap<string, number>;
  onToggle: (id: string) => void;
  chosenId?: string | null;
  onChoose?: (id: string) => void;
  /** Under a filter: the nodes shown, all of them open. */
  shown?: ReadonlySet<string> | null;
  /** Under a filter: the nodes it picks out; the others are muted. */
  matching?: ReadonlySet<string> | null;
}) {
  const list = useRef<HTMLDivElement>(null);
  // The chosen row scrolls into view, to the nearest edge and only when it is out of sight.
  useEffect(() => {
    if (!chosenId) return;
    for (const row of list.current?.querySelectorAll<HTMLElement>("[data-row]") ?? []) {
      if (row.dataset.row === chosenId) row.scrollIntoView?.({ block: "nearest" });
    }
  }, [chosenId, open]);
  const children = childrenOf(shown ? nodes.filter((node) => shown.has(node.id)) : nodes);
  const root = (children.get(null) ?? [])[0];
  if (!root) return null;
  const titles = new Map(nodes.map((node) => [node.id, shortTitle(node)]));
  // What waits on a node and everything under it.
  const within = (node: TreeNode): number =>
    (waiting.get(node.id) ?? 0) + (children.get(node.id) ?? []).reduce((sum, kid) => sum + within(kid), 0);
  const rows: React.ReactNode[] = [];
  const walk = (node: TreeNode, depth: number) => {
    const kids = children.get(node.id) ?? [];
    const isRoot = depth === 0;
    const expanded = isRoot || !!shown || open.has(node.id);
    const struck = node.status === "out_of_scope";
    rows.push(
      <div
        key={node.id}
        data-row={node.id}
        aria-current={node.id === chosenId || undefined}
        onClick={() => onChoose?.(node.id)}
        title={node.title}
        style={{ paddingLeft: depth > 1 ? (depth - 1) * 14 : 0 }}
        className={cn(
          "flex h-7 items-center gap-1 rounded-md pr-1.5 text-[12.5px] text-sidebar-foreground transition-colors hover:bg-sidebar-accent hover:text-foreground",
          "cursor-pointer",
          matching && !matching.has(node.id) && "opacity-50",
          node.id === chosenId && "bg-sidebar-accent text-foreground",
          depth === 0 && "h-[30px] text-[13px] font-semibold text-foreground",
          depth === 1 && "font-medium text-foreground",
        )}
      >
        {kids.length > 0 && !isRoot ? (
          <button
            type="button"
            aria-expanded={expanded}
            aria-label={expanded ? "Close" : "Open"}
            onClick={(event) => {
              event.stopPropagation();
              onToggle(node.id);
            }}
            className="grid h-[22px] w-[18px] shrink-0 place-items-center rounded text-muted-foreground"
          >
            <ChevronRight aria-hidden className={cn("size-2.5 transition-transform", expanded && "rotate-90")} />
          </button>
        ) : (
          <span aria-hidden className="w-[18px] shrink-0" />
        )}
        <span className={cn("min-w-0 flex-1 truncate", struck && "text-muted-foreground line-through")}>
          {isRoot ? node.title : shortTitle(node)}
        </span>
        {!isRoot && (
          <Marks
            node={node}
            waiting={waiting}
            count={kids.length > 0 && !expanded ? within(node) : undefined}
            countTitle="Open questions here and inside"
            titleOf={titles}
          />
        )}
      </div>,
    );
    if (expanded) for (const kid of kids) walk(kid, depth + 1);
  };
  walk(root, 0);
  return <div ref={list} className="min-h-0 flex-1 overflow-y-auto px-2 pb-20">{rows}</div>;
}
