// @vitest-environment jsdom
// US-224: the camera of the Behaviour screen's graph as plain functions. The
// zoom stops at 1.2% and at 200% with the point under the pointer kept still,
// and a move of the view is skipped when the system asks for reduced motion and
// stops on any new input.
import { afterEach, describe, expect, test, vi } from "vitest";
import { animateTo, zoomAt } from "./components/behaviour/camera";

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
