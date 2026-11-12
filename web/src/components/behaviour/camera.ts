import type { Rect } from "./layout";

/** The view: where the drawing's origin lies in the graph area and how much it is magnified. */
export type View = { x: number; y: number; k: number };

export const MIN_ZOOM = 0.012;
export const MAX_ZOOM = 2;

/** The view that shows all of `bounds` inside `area`, centred, with room for the legend above and the controls below. */
export function fit(bounds: Rect, area: { w: number; h: number }, maxZoom = 1): View {
  const side = 40;
  const top = 84;
  const bottom = 64;
  const k = Math.max(MIN_ZOOM, Math.min((area.w - side * 2) / bounds.w, (area.h - top - bottom) / bounds.h, maxZoom));
  return {
    k,
    x: (area.w - bounds.w * k) / 2 - bounds.x * k,
    y: top + (area.h - top - bottom - bounds.h * k) / 2 - bounds.y * k,
  };
}
