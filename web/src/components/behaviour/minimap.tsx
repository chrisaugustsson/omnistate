import { useEffect, useRef } from "react";
import type { View } from "./camera";
import type { Rect } from "./layout";

// The small map of the whole drawing: every box, the waiting ones in the signal colour, and a
// frame around what the graph area shows. Pressing or dragging in it moves the graph there.

const MAX_W = 150;
const MAX_H = 170;

type Props = {
  boxes: ReadonlyMap<string, Rect>;
  bounds: Rect;
  waiting: ReadonlySet<string>;
  view: View;
  area: { w: number; h: number };
  /** Called with the point of the drawing that was pressed. */
  onGo: (x: number, y: number) => void;
};

export function Minimap({ boxes, bounds, waiting, view, area, onGo }: Props) {
  const ref = useRef<HTMLCanvasElement>(null);
  const scale = bounds.w && bounds.h ? Math.min(MAX_W / bounds.w, MAX_H / bounds.h) : 1;
  const w = Math.max(24, Math.round(bounds.w * scale));
  const h = Math.max(24, Math.round(bounds.h * scale));
  useEffect(() => {
    const canvas = ref.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const css = getComputedStyle(canvas);
    const signal = css.getPropertyValue("--signal");
    const ink = css.getPropertyValue("--foreground");
    const bar = css.getPropertyValue("--muted-foreground");
    ctx.globalAlpha = 0.45;
    ctx.fillStyle = bar;
    for (const [id, box] of boxes) {
      if (!waiting.has(id)) ctx.fillRect((box.x - bounds.x) * scale, (box.y - bounds.y) * scale, Math.max(1.5, box.w * scale), Math.max(1, box.h * scale));
    }
    ctx.globalAlpha = 1;
    ctx.fillStyle = signal;
    for (const [id, box] of boxes) {
      if (waiting.has(id)) ctx.fillRect((box.x - bounds.x) * scale - 1, (box.y - bounds.y) * scale - 1, Math.max(1.5, box.w * scale) + 2, Math.max(1, box.h * scale) + 2);
    }
    ctx.strokeStyle = ink;
    ctx.lineWidth = 1;
    ctx.strokeRect(
      Math.max(0.5, (-view.x / view.k - bounds.x) * scale),
      Math.max(0.5, (-view.y / view.k - bounds.y) * scale),
      Math.min(w - 1, (area.w / view.k) * scale),
      Math.min(h - 1, (area.h / view.k) * scale),
    );
  }, [boxes, bounds, waiting, view, area, scale, w, h]);
  const go = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const at = event.currentTarget.getBoundingClientRect();
    onGo((event.clientX - at.left) / scale + bounds.x, (event.clientY - at.top) / scale + bounds.y);
  };
  return (
    <div data-no-pan className="absolute bottom-3 right-3 rounded-lg border bg-background p-1 leading-[0]">
      <canvas
        ref={ref}
        aria-label="Map of the tree"
        className="cursor-pointer touch-none"
        style={{ width: w, height: h }}
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId);
          go(event);
        }}
        onPointerMove={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) go(event);
        }}
      />
    </div>
  );
}
