import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { TreeNode } from "@/api/behaviour";
import { fit, type View } from "./camera";
import { elbow, layoutBlocks, planColumns } from "./layout";
import { shortTitle } from "./tree";

// The graph: every node but the root as a box, one block per story, drawn with absolutely
// placed divs and one svg inside a single container that carries the view.

const NO_FOLDS: ReadonlySet<string> = new Set();

/** The tree of `nodes` as story blocks, fitted into the area it stands in when it opens. */
export function Graph({ nodes }: { nodes: TreeNode[] }) {
  const ref = useRef<HTMLDivElement>(null);
  const [area, setArea] = useState<{ w: number; h: number } | null>(null);
  const [view, setView] = useState<View | null>(null);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => setArea({ w: element.clientWidth, h: element.clientHeight });
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const columns = useMemo(() => (area ? planColumns(nodes, null, area) : []), [nodes, area]);
  const { boxes, bounds } = useMemo(() => layoutBlocks(nodes, { shown: null, folded: NO_FOLDS, columns }), [nodes, columns]);
  useEffect(() => {
    if (area && area.w > 0 && area.h > 0 && boxes.size && !view) setView(fit(bounds, area));
  }, [area, boxes, bounds, view]);
  const byId = useMemo(() => new Map(nodes.map((node) => [node.id, node])), [nodes]);
  const lines: { id: string; d: string }[] = [];
  for (const [id, box] of boxes) {
    const parent = byId.get(id)?.parent_id;
    const above = parent ? boxes.get(parent) : undefined;
    if (above) lines.push({ id, d: elbow(above, box) });
  }
  return (
    <div ref={ref} className="relative size-full overflow-hidden">
      {view && (
        <div
          className="absolute left-0 top-0 origin-top-left"
          style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})` }}
        >
          <svg
            aria-hidden
            className="absolute left-0 top-0 text-border"
            width={Math.ceil(bounds.x + bounds.w + 400)}
            height={Math.ceil(bounds.y + bounds.h + 400)}
          >
            {lines.map((line) => (
              <path key={line.id} d={line.d} fill="none" stroke="currentColor" strokeWidth={1.5} />
            ))}
          </svg>
          {nodes.map((node) => {
            const box = boxes.get(node.id);
            if (!box) return null;
            return (
              <div
                key={node.id}
                data-node={node.id}
                className="absolute left-0 top-0 flex items-center rounded-lg border bg-background px-[9px] text-[12.5px] leading-[1.28]"
                style={{ width: box.w, height: box.h, transform: `translate(${box.x}px, ${box.y}px)` }}
              >
                <span className="line-clamp-2 min-w-0">{shortTitle(node)}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
