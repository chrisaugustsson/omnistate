import { ChevronRight } from "lucide-react";
import type { TreeNode } from "@/api/behaviour";
import { childrenOf, shortTitle } from "./tree";
import { cn } from "@/lib/utils";

/**
 * The tree as folders: the feature first, always open, then its stories and
 * what lies under them. `open` holds the ids of the nodes whose children show.
 */
export function Folders({
  nodes,
  open,
  onToggle,
}: {
  nodes: TreeNode[];
  open: ReadonlySet<string>;
  onToggle: (id: string) => void;
}) {
  const children = childrenOf(nodes);
  const root = (children.get(null) ?? [])[0];
  if (!root) return null;
  const rows: React.ReactNode[] = [];
  const walk = (node: TreeNode, depth: number) => {
    const kids = children.get(node.id) ?? [];
    const isRoot = depth === 0;
    const expanded = isRoot || open.has(node.id);
    const struck = node.status === "out_of_scope";
    rows.push(
      <div
        key={node.id}
        title={node.title}
        style={{ paddingLeft: depth > 1 ? (depth - 1) * 14 : 0 }}
        className={cn(
          "flex h-7 items-center gap-1 rounded-md pr-1.5 text-[12.5px] text-sidebar-foreground transition-colors hover:bg-sidebar-accent hover:text-foreground",
          depth === 0 && "h-[30px] text-[13px] font-semibold text-foreground",
          depth === 1 && "font-medium text-foreground",
        )}
      >
        {kids.length > 0 && !isRoot ? (
          <button
            type="button"
            aria-expanded={expanded}
            aria-label={expanded ? "Close" : "Open"}
            onClick={() => onToggle(node.id)}
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
      </div>,
    );
    if (expanded) for (const kid of kids) walk(kid, depth + 1);
  };
  walk(root, 0);
  return <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-20">{rows}</div>;
}
