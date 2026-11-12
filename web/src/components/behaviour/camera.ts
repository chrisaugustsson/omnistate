import type { Rect } from "./layout";

/** The view: where the drawing's origin lies in the graph area and how much it is magnified. */
export type View = { x: number; y: number; k: number };

export const MIN_ZOOM = 0.012;
export const MAX_ZOOM = 2;

/** The view that shows all of `bounds` inside `area`, centred, with room for the legend above and the controls below. */
export function fit(
  bounds: Rect,
  area: { w: number; h: number },
  maxZoom = 1,
  { side, top, bottom } = { side: 40, top: 84, bottom: 64 },
): View {
  const k = Math.max(MIN_ZOOM, Math.min((area.w - side * 2) / bounds.w, (area.h - top - bottom) / bounds.h, maxZoom));
  return {
    k,
    x: (area.w - bounds.w * k) / 2 - bounds.x * k,
    y: top + (area.h - top - bottom - bounds.h * k) / 2 - bounds.y * k,
  };
}

/** The view that shows one story's block: 64 clear on each side and 30 more above. */
export const fitBlock = (bounds: Rect, area: { w: number; h: number }) => fit(bounds, area, 1, { side: 64, top: 94, bottom: 64 });

const clampZoom = (k: number) => Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, k));

/** `view` magnified by `factor`, with the drawing's point under `point` (in the graph area) staying under it. */
export function zoomAt(view: View, point: { x: number; y: number }, factor: number): View {
  const k = clampZoom(view.k * factor);
  const wx = (point.x - view.x) / view.k;
  const wy = (point.y - view.y) / view.k;
  return { k, x: point.x - wx * k, y: point.y - wy * k };
}

/** `view` moved so that the middle of `box` is the middle of `area`; the zoom stays. */
export function centreOn(view: View, box: Rect, area: { w: number; h: number }): View {
  return { k: view.k, x: area.w / 2 - (box.x + box.w / 2) * view.k, y: area.h / 2 - (box.y + box.h / 2) * view.k };
}

/**
 * Where the view goes to bring `box` into sight, or null when it stays. Below 55% zoom a story is
 * fitted with its whole block and any other box is centred at 90%; from 55% a box lying inside
 * `area` with 64 to spare stays, and any other box is centred at the same zoom.
 */
export function bringIntoView(
  view: View,
  box: Rect,
  area: { w: number; h: number },
  { isStory, blockBounds }: { isStory: boolean; blockBounds?: Rect },
): View | null {
  if (view.k < 0.55) {
    if (isStory && blockBounds) return fitBlock(blockBounds, area);
    return centreOn({ ...view, k: 0.9 }, box, area);
  }
  const m = 64;
  const sx = view.x + box.x * view.k;
  const sy = view.y + box.y * view.k;
  if (sx >= m && sy >= m + 30 && sx + box.w * view.k <= area.w - m && sy + box.h * view.k <= area.h - m) return null;
  return centreOn(view, box, area);
}

// cubic-bezier(0.23, 1, 0.32, 1), solved for y at x by Newton steps.
const P1X = 0.23;
const P1Y = 1;
const P2X = 0.32;
const P2Y = 1;
export function ease(x: number): number {
  const cx = 3 * P1X;
  const bx = 3 * (P2X - P1X) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * P1Y;
  const by = 3 * (P2Y - P1Y) - cy;
  const ay = 1 - cy - by;
  let t = x;
  for (let i = 0; i < 8; i++) {
    const e = ((ax * t + bx) * t + cx) * t - x;
    const d = (3 * ax * t + 2 * bx) * t + cx;
    if (Math.abs(e) < 1e-5 || !d) break;
    t -= e / d;
  }
  return ((ay * t + by) * t + cy) * t;
}

/**
 * Moves from `from` to `target` over `ms`, calling `apply` with each step; returns a function that stops it.
 * Lands at once with no steps in between when `ms` is 0 or the system asks for reduced motion.
 */
export function animateTo(from: View, target: View, ms: number, apply: (view: View) => void): () => void {
  const reduced = typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (!ms || reduced) {
    apply(target);
    return () => {};
  }
  const start = performance.now();
  let frame = 0;
  const stop = () => {
    cancelAnimationFrame(frame);
    clearTimeout(landing);
  };
  // Lands on the target even where frames are not being drawn.
  const landing = setTimeout(() => {
    cancelAnimationFrame(frame);
    apply(target);
  }, ms + 120);
  const step = (now: number) => {
    const u = Math.max(0, Math.min(1, (now - start) / ms));
    const e = ease(u);
    apply({ x: from.x + (target.x - from.x) * e, y: from.y + (target.y - from.y) * e, k: from.k + (target.k - from.k) * e });
    if (u < 1) frame = requestAnimationFrame(step);
    else clearTimeout(landing);
  };
  frame = requestAnimationFrame(step);
  return stop;
}
