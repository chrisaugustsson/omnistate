import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { TreeNode } from "@/api/behaviour";
import { cn } from "@/lib/utils";
import { animateTo, bringIntoView, centreOn, fit, fitBlock, tween, zoomAt, type View } from "./camera";
import { elbow, layoutBlocks, planColumns, type Rect } from "./layout";
import { Minimap } from "./minimap";
import { Legend, Marks } from "./marks";
import { ancestorsOf, childrenOf, descendantsOf, shortTitle } from "./tree";

// The graph: every node but the root as a box, one block per story, drawn with absolutely
// placed divs and one svg inside a single container that carries the view.

const NO_FOLDS: ReadonlySet<string> = new Set();
const NONE: ReadonlyMap<string, number> = new Map();
const BARS_BELOW = 0.3;
const LABELS_BELOW = 0.46;
const BUTTON_ZOOM = 1.4;
const FOLD_MS = 220;
// A press that moves less than this is a click, not a drag.
const DRAG_FROM = 4;

/** A fold's slide under way: where the boxes started and the eased progress. */
type Slide = { from: ReadonlyMap<string, Rect>; e: number } | null;

/** The tree of `nodes` as story blocks, fitted into the area it stands in when it opens. */
export function Graph({
  nodes,
  waiting = NONE,
  chosenId = null,
  onChoose,
  reveal = null,
  shown = null,
  matching = null,
  found = null,
  fitN = 0,
  fitMs = 0,
  folded = NO_FOLDS,
  onFold,
}: {
  nodes: TreeNode[];
  /** How many questions wait on each node. */
  waiting?: ReadonlyMap<string, number>;
  chosenId?: string | null;
  /** A box was pressed. */
  onChoose?: (id: string) => void;
  /** A request, new each time, to bring a chosen node into sight over `ms`. */
  reveal?: { id: string; ms: number; n: number } | null;
  /** Under a filter: the nodes drawn. */
  shown?: ReadonlySet<string> | null;
  /** Under a filter: the nodes it picks out; the others are muted. */
  matching?: ReadonlySet<string> | null;
  /** While a search is typed: the nodes found; the others are dimmed. */
  found?: ReadonlySet<string> | null;
  /** Changes each time the whole drawing is to be fitted again. */
  fitN?: number;
  /** How long that fit takes, in ms. */
  fitMs?: number;
  /** The nodes whose children are folded away. */
  folded?: ReadonlySet<string>;
  /** A fold pill was pressed. */
  onFold?: (id: string) => void;
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
  const columns = useMemo(() => (area ? planColumns(nodes, shown, area) : []), [nodes, shown, area]);
  const { boxes, bounds, blocks } = useMemo(() => layoutBlocks(nodes, { shown, folded, columns }), [nodes, shown, folded, columns]);
  const byId = useMemo(() => new Map(nodes.map((node) => [node.id, node])), [nodes]);
  // While a fold moves, the boxes slide from where they stood in the same frames as the view, so the pressed box stays still.
  const [slide, setSlideState] = useState<Slide>(null);
  const slideRef = useRef<Slide>(null);
  const setSlide = (next: Slide) => {
    slideRef.current = next;
    setSlideState(next);
  };
  const placeAt = useCallback((slide: Slide) => {
    if (!slide) return boxes;
    const at = new Map<string, Rect>();
    for (const [id, box] of boxes) {
      // A box coming back starts where the nearest box above it stood.
      let from = slide.from.get(id);
      for (let up = byId.get(id)?.parent_id; !from && up; up = byId.get(up)?.parent_id) from = slide.from.get(up);
      from ??= box;
      at.set(id, { ...box, x: from.x + (box.x - from.x) * slide.e, y: from.y + (box.y - from.y) * slide.e });
    }
    return at;
  }, [boxes, byId]);
  const placed = useMemo(() => placeAt(slide), [placeAt, slide]);
  useEffect(() => {
    if (area && area.w > 0 && area.h > 0 && boxes.size && !view) setView(fit(bounds, area));
  }, [area, boxes, bounds, view]);
  const areaRef = useRef(area);
  areaRef.current = area;
  const boundsRef = useRef(bounds);
  boundsRef.current = bounds;
  // A new filter fits the whole drawing again.
  const fitted = useRef(fitN);
  useEffect(() => {
    if (fitted.current === fitN) return;
    fitted.current = fitN;
    if (areaRef.current && areaRef.current.w > 0 && boundsRef.current.w > 0) moveTo(fit(boundsRef.current, areaRef.current), fitMs);
  }, [fitN, fitMs, moveTo]);
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
  // Under a filter only the shown nodes count as children.
  const below = useMemo(() => {
    const kids = childrenOf(nodes);
    const count = (id: string): { n: number; waits: boolean; hits: number } => {
      let n = 0;
      let waits = false;
      let hits = 0;
      for (const child of kids.get(id) ?? []) {
        if (shown && !shown.has(child.id)) continue;
        const inner = count(child.id);
        n += 1 + inner.n;
        waits ||= inner.waits || waiting.has(child.id);
        hits += (found?.has(child.id) ? 1 : 0) + inner.hits;
      }
      return { n, waits, hits };
    };
    const kidsShown = (id: string) => (kids.get(id) ?? []).some((child) => !shown || shown.has(child.id));
    return { count, kidsShown };
  }, [nodes, shown, waiting, found]);
  // Folding keeps the pressed box where it is on screen: the view moves as far as the box does, in the same time.
  const pressFold = (id: string) => {
    const current = viewRef.current;
    // Where the boxes stand now, in step with the view even between a frame's move and its render.
    const from = placeAt(slideRef.current);
    const before = from.get(id);
    if (current && before && areaRef.current) {
      const next = new Set(folded);
      if (!next.delete(id)) next.add(id);
      const after = layoutBlocks(nodes, { shown, folded: next, columns }).boxes.get(id);
      if (after) {
        const target = { k: current.k, x: current.x + (before.x - after.x) * current.k, y: current.y + (before.y - after.y) * current.k };
        stopMove.current();
        // The boxes stand where they were in the same render that applies the fold, before the first frame.
        setSlide({ from, e: 0 });
        const stop = tween(FOLD_MS, (e) => {
          setView(e === 1 ? target : { k: current.k, x: current.x + (target.x - current.x) * e, y: current.y + (target.y - current.y) * e });
          setSlide(e === 1 ? null : { from, e });
        });
        stopMove.current = () => {
          stop();
          setSlide(null);
        };
      }
    }
    onFold?.(id);
  };
  const fitAll = (ms: number) => {
    if (areaRef.current) moveTo(fit(boundsRef.current, areaRef.current), ms);
  };
  const fitBoxAndBelow = (id: string) => {
    const ids = [id, ...descendantsOf(nodes, id)];
    const shown = ids.map((one) => boxes.get(one)).filter((box) => !!box);
    if (!shown.length || !areaRef.current) return;
    const x = Math.min(...shown.map((box) => box.x));
    const y = Math.min(...shown.map((box) => box.y));
    const right = Math.max(...shown.map((box) => box.x + box.w));
    const bottom = Math.max(...shown.map((box) => box.y + box.h));
    moveTo(fitBlock({ x, y, w: right - x, h: bottom - y }, areaRef.current), 260);
  };
  const fitBlockOf = (id: string) => {
    const block = blocks.get(id);
    if (block && areaRef.current) moveTo(fitBlock(block, areaRef.current), 260);
  };

  // A node chosen outside the graph is brought into sight by the rules of bringIntoView.
  const handled = useRef<number | null>(null);
  useEffect(() => {
    const current = viewRef.current;
    if (!reveal || !current || !areaRef.current || handled.current === reveal.n) return;
    handled.current = reveal.n;
    const box = boxes.get(reveal.id);
    // The feature's root has no box: choosing it fits the whole drawing.
    if (!box) {
      moveTo(fit(boundsRef.current, areaRef.current), reveal.ms);
      return;
    }
    const target = bringIntoView(current, box, areaRef.current, { isStory: blocks.has(reveal.id), blockBounds: blocks.get(reveal.id) });
    if (target) moveTo(target, reveal.ms);
  }, [reveal, view, boxes, blocks, moveTo]);

  // Pan by dragging, zoom by wheel, pinch and the key F.
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const gesture = useRef<{ x: number; y: number; view: View; moved: boolean; id?: string; pinch?: { distance: number; view: View } } | null>(null);
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
      const pressed = (event.target as HTMLElement).closest<HTMLElement>("[data-node]");
      gesture.current = { ...local(event), view: current, moved: false, id: pressed?.dataset.node };
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
  const lastPress = useRef<{ id: string; at: number } | null>(null);
  const onPointerEnd = (event: React.PointerEvent<HTMLDivElement>) => {
    pointers.current.delete(event.pointerId);
    const g = gesture.current;
    // A press that did not drag chooses its box; a second press on it soon after fits it with all under it.
    if (event.type === "pointerup" && g?.id && !g.moved && !g.pinch && pointers.current.size === 0) {
      const now = performance.now();
      if (lastPress.current && lastPress.current.id === g.id && now - lastPress.current.at < 400) {
        lastPress.current = null;
        fitBoxAndBelow(g.id);
      } else {
        lastPress.current = { id: g.id, at: now };
      }
      if (g.id !== chosenId) onChoose?.(g.id);
    }
    if (!pointers.current.size) gesture.current = null;
  };
  const goTo = (x: number, y: number) => {
    const current = viewRef.current;
    if (!current || !areaRef.current) return;
    stopMove.current();
    setView(centreOn(current, { x, y, w: 0, h: 0 }, areaRef.current));
  };
  const far = !!view && view.k < BARS_BELOW;
  const titles = useMemo(() => new Map(nodes.map((node) => [node.id, shortTitle(node)])), [nodes]);
  // The lines from the chosen node up to its story are drawn in ink, above the rest; none while a fold hides it.
  const inkIds = useMemo(() => {
    if (!chosenId || !boxes.has(chosenId)) return new Set<string>();
    const above = ancestorsOf(nodes, chosenId);
    return new Set([chosenId, ...above.slice(0, -1)]);
  }, [nodes, chosenId, boxes]);
  const lines: { id: string; d: string }[] = [];
  for (const [id, box] of placed) {
    const parent = byId.get(id)?.parent_id;
    const above = parent ? placed.get(parent) : undefined;
    if (above) lines.push({ id, d: elbow(above, box) });
  }
  lines.sort((a, b) => Number(inkIds.has(a.id)) - Number(inkIds.has(b.id)));
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
              <path
                key={line.id}
                d={line.d}
                fill="none"
                stroke={inkIds.has(line.id) ? "var(--foreground)" : "currentColor"}
                strokeWidth={inkIds.has(line.id) ? 2 : 1.5}
              />
            ))}
          </svg>
          {nodes.map((node) => {
            const box = placed.get(node.id);
            if (!box) return null;
            return (
              <div
                key={node.id}
                data-node={node.id}
                data-waiting={waiting.has(node.id) || undefined}
                data-pass={node.needs_pass || undefined}
                data-status={node.status}
                data-chosen={node.id === chosenId || undefined}
                className={cn(
                  (found ? !found.has(node.id) : matching && !matching.has(node.id)) && "opacity-50",
                  node.id === chosenId && "ring-2 ring-foreground ring-offset-1 ring-offset-background",
                  "absolute left-0 top-0 flex items-center gap-1.5 rounded-lg border bg-background px-[9px] text-[12.5px] leading-[1.28] group-data-[far]:rounded-[3px] group-data-[far]:border-transparent group-data-[far]:bg-muted-foreground/35",
                  node.status === "out_of_scope" && "border-dashed bg-transparent text-muted-foreground group-data-[far]:border group-data-[far]:border-dashed group-data-[far]:border-muted-foreground group-data-[far]:bg-transparent",
                  node.needs_pass && "border-dashed border-foreground group-data-[far]:border-transparent group-data-[far]:bg-foreground",
                  waiting.has(node.id) && !node.needs_pass && "group-data-[far]:bg-signal",
                  "transition-transform duration-[220ms] ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
                )}
                style={{ width: box.w, height: box.h, transform: `translate(${box.x}px, ${box.y}px)`, transition: slide ? "none" : undefined }}
              >
                {node.status === "open" && (waiting.has(node.id) || !node.needs_pass) && (
                  <span aria-hidden data-open-ring className="size-2 shrink-0 rounded-full border-2 border-signal group-data-[far]:invisible" />
                )}
                <span className={cn("line-clamp-2 min-w-0 flex-1 group-data-[far]:invisible", node.status === "out_of_scope" && "line-through")}>
                  {shortTitle(node)}
                </span>
                <span className="flex shrink-0 items-center gap-1 group-data-[far]:invisible">
                  <Marks node={node} waiting={waiting} titleOf={titles} />
                </span>
                {below.kidsShown(node.id) && (
                  <FoldPill folded={folded.has(node.id)} hidden={below.count(node.id)} showHits={!!found} onPress={() => pressFold(node.id)} />
                )}
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
      {!shown && <Legend />}
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

/** The pill at a box's right edge: "−" folds what lies under it, "+N" shows the N hidden nodes again. */
function FoldPill({ folded, hidden, showHits, onPress }: { folded: boolean; hidden: { n: number; waits: boolean; hits: number }; showHits: boolean; onPress: () => void }) {
  return (
    <button
      type="button"
      data-no-pan
      data-fold
      aria-expanded={!folded}
      title={folded ? `Show the ${hidden.n} nodes under this one` : "Fold what lies under this node"}
      onClick={onPress}
      className="absolute left-full top-1/2 ml-1.5 -mt-[9px] flex h-[18px] min-w-[18px] cursor-pointer items-center justify-center gap-[3px] whitespace-nowrap rounded-full border border-input bg-background px-[5px] text-[10.5px] font-medium leading-4 tabular-nums text-muted-foreground hover:border-muted-foreground hover:text-foreground active:scale-95"
    >
      {folded ? `+${hidden.n}${showHits && hidden.hits > 0 ? ` · ${hidden.hits} found` : ""}` : "−"}
      {folded && hidden.waits && <span aria-hidden data-fold-dot className="size-1.5 rounded-full bg-signal" />}
    </button>
  );
}
