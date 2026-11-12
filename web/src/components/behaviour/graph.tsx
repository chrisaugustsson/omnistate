import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { TreeNode } from "@/api/behaviour";
import { animateTo, centreOn, fit, fitBlock, zoomAt, type View } from "./camera";
import { elbow, layoutBlocks, planColumns } from "./layout";
import { Minimap } from "./minimap";
import { shortTitle } from "./tree";

// The graph: every node but the root as a box, one block per story, drawn with absolutely
// placed divs and one svg inside a single container that carries the view.

const NO_FOLDS: ReadonlySet<string> = new Set();
const NONE: ReadonlySet<string> = new Set();
const BARS_BELOW = 0.3;
const LABELS_BELOW = 0.46;
const BUTTON_ZOOM = 1.4;
// A press that moves less than this is a click, not a drag.
const DRAG_FROM = 4;

/** The tree of `nodes` as story blocks, fitted into the area it stands in when it opens. */
export function Graph({
  nodes,
  waiting = NONE,
  chosenId = null,
}: {
  nodes: TreeNode[];
  /** The nodes with a question waiting for an answer. */
  waiting?: ReadonlySet<string>;
  chosenId?: string | null;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [area, setArea] = useState<{ w: number; h: number } | null>(null);
  const [view, setViewState] = useState<View | null>(null);
  const viewRef = useRef<View | null>(null);
  const stopMove = useRef<() => void>(() => {});
  const setView = useCallback((next: View) => {
    viewRef.current = next;
    setViewState(next);
  }, []);
  // Any new input stops a move under way; `ms` 0 is an immediate jump.
  const moveTo = useCallback(
    (target: View, ms = 0) => {
      stopMove.current();
      stopMove.current = animateTo(viewRef.current ?? target, target, ms, setView);
    },
    [setView],
  );
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
  const { boxes, bounds, blocks } = useMemo(() => layoutBlocks(nodes, { shown: null, folded: NO_FOLDS, columns }), [nodes, columns]);
  useEffect(() => {
    if (area && area.w > 0 && area.h > 0 && boxes.size && !view) setView(fit(bounds, area));
  }, [area, boxes, bounds, view]);
  const areaRef = useRef(area);
  areaRef.current = area;
  const boundsRef = useRef(bounds);
  boundsRef.current = bounds;
  const middle = () => ({ x: (areaRef.current?.w ?? 0) / 2, y: (areaRef.current?.h ?? 0) / 2 });
  const zoomBy = (factor: number) => {
    const current = viewRef.current;
    if (current) moveTo(zoomAt(current, middle(), factor), 160);
  };
  const zoomTo100 = () => {
    const current = viewRef.current;
    const box = chosenId ? boxes.get(chosenId) : undefined;
    if (!current || !areaRef.current) return;
    if (box) moveTo(centreOn({ ...current, k: 1 }, box, areaRef.current), 240);
    else moveTo(zoomAt(current, middle(), 1 / current.k), 160);
  };
  const fitAll = (ms: number) => {
    if (areaRef.current) moveTo(fit(boundsRef.current, areaRef.current), ms);
  };
  const fitBlockOf = (id: string) => {
    const block = blocks.get(id);
    if (block && areaRef.current) moveTo(fitBlock(block, areaRef.current), 260);
  };

  // Pan by dragging, zoom by wheel, pinch and the key F.
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const gesture = useRef<{ x: number; y: number; view: View; moved: boolean; pinch?: { distance: number; view: View } } | null>(null);
  const local = (event: { clientX: number; clientY: number }) => {
    const at = ref.current!.getBoundingClientRect();
    return { x: event.clientX - at.left, y: event.clientY - at.top };
  };
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const current = viewRef.current;
      if (!current) return;
      stopMove.current();
      if (event.ctrlKey || event.metaKey) moveTo(zoomAt(current, local(event), Math.exp(-event.deltaY * 0.0125)));
      else moveTo({ ...current, x: current.x - event.deltaX, y: current.y - event.deltaY });
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    const onKey = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "f" || event.metaKey || event.ctrlKey || event.altKey) return;
      const target = event.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) return;
      fitAll(0);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      element.removeEventListener("wheel", onWheel);
      window.removeEventListener("keydown", onKey);
    };
  }, [moveTo]);
  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    const current = viewRef.current;
    if (event.button !== 0 || !current || (event.target as HTMLElement).closest("[data-no-pan]")) return;
    stopMove.current();
    event.currentTarget.setPointerCapture(event.pointerId);
    pointers.current.set(event.pointerId, local(event));
    if (pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      gesture.current = { x: 0, y: 0, view: current, moved: true, pinch: { distance: Math.hypot(a.x - b.x, a.y - b.y), view: current } };
    } else {
      gesture.current = { ...local(event), view: current, moved: false };
    }
  };
  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const g = gesture.current;
    if (!g || !pointers.current.has(event.pointerId)) return;
    pointers.current.set(event.pointerId, local(event));
    if (g.pinch && pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      const distance = Math.hypot(a.x - b.x, a.y - b.y);
      if (g.pinch.distance > 0) setView(zoomAt(g.pinch.view, { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }, distance / g.pinch.distance));
      return;
    }
    const at = local(event);
    const dx = at.x - g.x;
    const dy = at.y - g.y;
    if (!g.moved && Math.hypot(dx, dy) < DRAG_FROM) return;
    g.moved = true;
    setView({ ...g.view, x: g.view.x + dx, y: g.view.y + dy });
  };
  const onPointerEnd = (event: React.PointerEvent<HTMLDivElement>) => {
    pointers.current.delete(event.pointerId);
    if (!pointers.current.size) gesture.current = null;
  };
  const goTo = (x: number, y: number) => {
    const current = viewRef.current;
    if (!current || !areaRef.current) return;
    stopMove.current();
    setView(centreOn(current, { x, y, w: 0, h: 0 }, areaRef.current));
  };
  const far = !!view && view.k < BARS_BELOW;
  const byId = useMemo(() => new Map(nodes.map((node) => [node.id, node])), [nodes]);
  const lines: { id: string; d: string }[] = [];
  for (const [id, box] of boxes) {
    const parent = byId.get(id)?.parent_id;
    const above = parent ? boxes.get(parent) : undefined;
    if (above) lines.push({ id, d: elbow(above, box) });
  }
  return (
    <div
      ref={ref}
      data-far={far || undefined}
      className="group relative size-full touch-none select-none overflow-hidden"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
    >
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
                data-waiting={waiting.has(node.id) || undefined}
                className="absolute left-0 top-0 flex items-center rounded-lg border bg-background px-[9px] text-[12.5px] leading-[1.28] group-data-[far]:rounded-[3px] group-data-[far]:border-transparent group-data-[far]:bg-muted-foreground/35 group-data-[far]:data-[waiting]:bg-signal"
                style={{ width: box.w, height: box.h, transform: `translate(${box.x}px, ${box.y}px)` }}
              >
                <span className="line-clamp-2 min-w-0 group-data-[far]:invisible">{shortTitle(node)}</span>
              </div>
            );
          })}
        </div>
      )}
      {view && area && view.k < LABELS_BELOW && (
        <div className="pointer-events-none absolute inset-0 overflow-hidden">
          {[...blocks].map(([id, block]) => {
            const story = byId.get(id);
            if (!story) return null;
            return (
              <button
                key={id}
                type="button"
                data-no-pan
                data-story-label={id}
                className="pointer-events-auto absolute left-0 top-0 -ml-[5px] flex h-[18px] cursor-pointer items-center truncate whitespace-nowrap rounded px-[5px] text-[11.5px] font-medium leading-[18px] hover:bg-sidebar-accent"
                style={{
                  transform: `translate(${view.x + block.x * view.k}px, ${view.y + block.y * view.k - 21}px)`,
                  maxWidth: Math.max(120, block.w * view.k),
                }}
                onClick={() => fitBlockOf(id)}
              >
                <span className="truncate">{shortTitle(story)}</span>
              </button>
            );
          })}
        </div>
      )}
      {view && area && (
        <>
          <div data-no-pan className="absolute bottom-3 left-3 flex flex-col overflow-hidden rounded-lg border bg-background text-sm text-muted-foreground">
            {[
              { label: "Zoom in", text: "+", onClick: () => zoomBy(BUTTON_ZOOM) },
              { label: "Zoom to 100%", text: `${Math.round(view.k * 100)}%`, onClick: zoomTo100, small: true },
              { label: "Zoom out", text: "−", onClick: () => zoomBy(1 / BUTTON_ZOOM) },
              { label: "Fit everything", text: "⤢", onClick: () => fitAll(260) },
            ].map((button) => (
              <button
                key={button.label}
                type="button"
                aria-label={button.label}
                title={button.label}
                onClick={button.onClick}
                className={`grid h-[30px] w-[34px] place-items-center border-t first:border-t-0 hover:bg-sidebar-accent hover:text-foreground ${button.small ? "text-[10.5px] tabular-nums" : ""}`}
              >
                {button.text}
              </button>
            ))}
          </div>
          <Minimap boxes={boxes} bounds={bounds} waiting={waiting} view={view} area={area} onGo={goTo} />
        </>
      )}
    </div>
  );
}
