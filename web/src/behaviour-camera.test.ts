// @vitest-environment jsdom
// US-224: the camera of the Behaviour screen's graph as plain functions. The
// zoom stops at 1.2% and at 200% with the point under the pointer kept still,
// and a move of the view is skipped when the system asks for reduced motion and
// stops on any new input.
import { afterEach, describe, expect, test, vi } from "vitest";
import { animateTo, bringIntoView, zoomAt } from "./components/behaviour/camera";

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("zoomAt", () => {
  test("stops at 200% and keeps the drawing's point under the pointer", () => {
    const view = zoomAt({ x: 0, y: 0, k: 1.5 }, { x: 300, y: 150 }, 1.4);

    // The drawing's point (200, 100) was under (300, 150) at 150%.
    expect(view.k).toBe(2);
    expect(view.x + 200 * view.k).toBeCloseTo(300);
    expect(view.y + 100 * view.k).toBeCloseTo(150);
  });

  test("stops at 1.2%", () => {
    const view = zoomAt({ x: 0, y: 0, k: 0.015 }, { x: 300, y: 150 }, 1 / 1.4);

    expect(view.k).toBeCloseTo(0.012, 6);
  });
});

describe("animateTo", () => {
  test("lands on the target at once and with no step in between when the system asks for reduced motion", () => {
    vi.stubGlobal("matchMedia", (query: string) => ({ matches: query === "(prefers-reduced-motion: reduce)" }));
    const apply = vi.fn();

    animateTo({ x: 0, y: 0, k: 1 }, { x: -400, y: 120, k: 0.5 }, 240, apply);

    expect(apply.mock.calls).toEqual([[{ x: -400, y: 120, k: 0.5 }]]);
  });

  test("moves the view no further once stopped", () => {
    vi.useFakeTimers();
    const apply = vi.fn();

    const stop = animateTo({ x: 0, y: 0, k: 1 }, { x: -400, y: 120, k: 0.5 }, 240, apply);
    stop();
    vi.advanceTimersByTime(1000);

    expect(apply).not.toHaveBeenCalled();
  });
});

// US-226: where the view goes when a node is chosen outside the graph, in a
// graph area of 748 x 844. Below 55% zoom a story is fitted with its block (64
// clear on each side, 30 more above, at most 100%) and any other box is centred
// at 90%; from 55% a box lying inside the area with 64 to spare stays, and any
// other box is centred at the same zoom.
describe("bringIntoView", () => {
  const AREA = { w: 748, h: 844 };
  const BOX = { w: 252, h: 30 };

  test("below 55% fits a story with its whole block and centres any other box at 90%", () => {
    const far = { x: 0, y: 0, k: 0.5 };
    const block = { x: 0, y: 0, w: 582, h: 500 };

    expect(bringIntoView(far, { x: 0, y: 235, w: 252, h: 46 }, AREA, { isStory: true, blockBounds: block })).toEqual({ k: 1, x: 83, y: 187 });
    const centred = bringIntoView(far, { x: 1000, y: 2000, ...BOX }, AREA, { isStory: false });
    expect(centred!.k).toBe(0.9);
    expect(centred!.x).toBeCloseTo(-639.4, 6);
    expect(centred!.y).toBeCloseTo(-1391.5, 6);
  });

  test("at 55% leaves a box inside the graph area where it is", () => {
    expect(bringIntoView({ x: 0, y: 0, k: 0.55 }, { x: 200, y: 400, ...BOX }, AREA, { isStory: false })).toBeNull();
  });

  test("from 55% leaves a box with 64 to spare at the side and below, and centres one with 63 at the same zoom", () => {
    const near = { x: 0, y: 0, k: 1 };

    expect(bringIntoView(near, { x: 64, y: 200, ...BOX }, AREA, { isStory: false })).toBeNull();
    expect(bringIntoView(near, { x: 63, y: 200, ...BOX }, AREA, { isStory: false })).toEqual({ k: 1, x: 185, y: 207 });
    expect(bringIntoView(near, { x: 432, y: 200, ...BOX }, AREA, { isStory: false })).toBeNull();
    expect(bringIntoView(near, { x: 433, y: 200, ...BOX }, AREA, { isStory: false })).toEqual({ k: 1, x: -185, y: 207 });
    expect(bringIntoView(near, { x: 200, y: 750, ...BOX }, AREA, { isStory: false })).toBeNull();
    expect(bringIntoView(near, { x: 200, y: 751, ...BOX }, AREA, { isStory: false })).toEqual({ k: 1, x: 48, y: -344 });
  });
});
