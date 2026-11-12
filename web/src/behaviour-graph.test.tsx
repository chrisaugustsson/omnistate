// @vitest-environment jsdom
// US-224: moving around the Behaviour screen's graph. The graph of the PRD's
// test tree is rendered in a graph area of 748 x 844 (the one a 1440 x 900
// window gives) and the view is read back from the drawing's transform:
// `translate(Xpx, Ypx) scale(K)` puts the drawing's point (px, py) at
// (X + px * K, Y + py * K) in the graph area.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { loadWaitingNodes } from "./api/behaviour";
import { Graph } from "./components/behaviour/graph";
import { testTree } from "./testing/behaviour-test-tree";

const AREA = { w: 748, h: 844 };
const MIDDLE = { x: 374, y: 422 };
const SET_UP_STORY = "Player: create an account and get set up";
const PICK_PROGRAM_STORY = "Player: pick a training program";

type View = { x: number; y: number; k: number };

/** The view the drawing is shown in now. */
function currentView(container: HTMLElement): View {
  const drawing = container.querySelector<HTMLElement>('[style*="scale("]')!;
  const [, x, y, k] = /translate\((-?[\d.e-]+)px, (-?[\d.e-]+)px\) scale\(([\d.e-]+)\)/.exec(drawing.style.transform)!;
  return { x: Number(x), y: Number(y), k: Number(k) };
}

/** The drawing's point that `view` shows at `point` of the graph area. */
function drawingPointAt(view: View, point: { x: number; y: number }) {
  return { x: (point.x - view.x) / view.k, y: (point.y - view.y) / view.k };
}

const zoomShown = () => screen.getByRole("button", { name: "Zoom to 100%" }).textContent;

/** Presses the percentage button and waits for the move to land at 100%. */
async function zoomTo100() {
  fireEvent.click(screen.getByRole("button", { name: "Zoom to 100%" }));
  await waitFor(() => expect(zoomShown()).toBe("100%"));
}

function graphArea(container: HTMLElement) {
  return container.firstElementChild as HTMLElement;
}

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(AREA.w);
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(AREA.h);
  const captured = new Set<number>();
  Element.prototype.setPointerCapture = (id: number) => void captured.add(id);
  Element.prototype.hasPointerCapture = (id: number) => captured.has(id);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("the graph's zoom controls", () => {
  test("the percentage button sets 100% about the middle of the graph area when nothing is chosen", async () => {
    const { container } = render(<Graph nodes={testTree()} />);
    const opening = currentView(container);

    await zoomTo100();

    const view = currentView(container);
    expect(view.k).toBeCloseTo(1, 6);
    expect(drawingPointAt(view, MIDDLE).x).toBeCloseTo(drawingPointAt(opening, MIDDLE).x, 6);
    expect(drawingPointAt(view, MIDDLE).y).toBeCloseTo(drawingPointAt(opening, MIDDLE).y, 6);
  });

  test("the percentage button sets 100% centred on the chosen node", async () => {
    const chosen = "player-picks-program/new-player-has-no-program";
    const { container } = render(<Graph nodes={testTree()} chosenId={chosen} />);
    const box = container.querySelector<HTMLElement>(`[data-node="${chosen}"]`)!;
    const [, bx, by] = /translate\((-?[\d.]+)px, (-?[\d.]+)px\)/.exec(box.style.transform)!;

    await zoomTo100();
    await waitFor(() => expect(currentView(container).k).toBeCloseTo(1, 6));

    const view = currentView(container);
    expect(view.x + Number(bx) + parseFloat(box.style.width) / 2).toBeCloseTo(MIDDLE.x, 6);
    expect(view.y + Number(by) + parseFloat(box.style.height) / 2).toBeCloseTo(MIDDLE.y, 6);
  });

  test("zoom in and zoom out multiply the zoom by 1.4 about the middle of the graph area, and the percentage follows", async () => {
    const { container } = render(<Graph nodes={testTree()} />);
    await zoomTo100();
    const at100 = currentView(container);

    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    await waitFor(() => expect(zoomShown()).toBe("140%"));
    await waitFor(() => expect(currentView(container).k).toBeCloseTo(1.4, 6));
    expect(drawingPointAt(currentView(container), MIDDLE).x).toBeCloseTo(drawingPointAt(at100, MIDDLE).x, 6);
    expect(drawingPointAt(currentView(container), MIDDLE).y).toBeCloseTo(drawingPointAt(at100, MIDDLE).y, 6);
  });

  test("the fit button eases back to the whole drawing rather than jumping", async () => {
    const { container } = render(<Graph nodes={testTree()} />);
    const opening = currentView(container);
    await zoomTo100();
    const at100 = currentView(container);

    fireEvent.click(screen.getByRole("button", { name: "Fit everything" }));

    expect(currentView(container)).toEqual(at100);
    await waitFor(() => expect(currentView(container)).toEqual(opening));
  });
});

describe("the key F", () => {
  test("shows the whole drawing at once", async () => {
    const { container } = render(<Graph nodes={testTree()} />);
    const opening = currentView(container);
    await zoomTo100();

    fireEvent.keyDown(window, { key: "f" });

    expect(currentView(container)).toEqual(opening);
  });

  test("does nothing while typing in a field or with ⌘, Ctrl or Alt held", async () => {
    const { container } = render(
      <>
        <Graph nodes={testTree()} />
        <input aria-label="Search the tree" />
      </>,
    );
    await zoomTo100();
    const at100 = currentView(container);

    fireEvent.keyDown(screen.getByRole("textbox", { name: "Search the tree" }), { key: "f" });
    fireEvent.keyDown(window, { key: "f", metaKey: true });
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    fireEvent.keyDown(window, { key: "f", altKey: true });

    expect(currentView(container)).toEqual(at100);
  });
});

describe("dragging and scrolling the graph", () => {
  test("a press that moves less than 4 pixels leaves the drawing still; from 4 the drawing follows the whole dragged distance", () => {
    const { container } = render(<Graph nodes={testTree()} />);
    const area = graphArea(container);
    const opening = currentView(container);

    fireEvent.pointerDown(area, { pointerId: 1, button: 0, clientX: 100, clientY: 100 });
    fireEvent.pointerMove(area, { pointerId: 1, clientX: 103, clientY: 100 });
    expect(currentView(container)).toEqual(opening);

    fireEvent.pointerMove(area, { pointerId: 1, clientX: 104, clientY: 100 });
    expect(currentView(container)).toEqual({ ...opening, x: opening.x + 4 });

    fireEvent.pointerMove(area, { pointerId: 1, clientX: 150, clientY: 130 });
    expect(currentView(container)).toEqual({ ...opening, x: opening.x + 50, y: opening.y + 30 });
  });

  test("scrolling moves the drawing by the scrolled distance", () => {
    const { container } = render(<Graph nodes={testTree()} />);
    const opening = currentView(container);

    fireEvent.wheel(graphArea(container), { deltaX: 30, deltaY: 50 });

    expect(currentView(container)).toEqual({ ...opening, x: opening.x - 30, y: opening.y - 50 });
  });

  test("scrolling with ⌘ or Ctrl held (a pinch comes as Ctrl) zooms in and keeps the point under the pointer still", () => {
    const { container } = render(<Graph nodes={testTree()} />);
    const opening = currentView(container);
    const pointer = { x: 200, y: 300 };

    fireEvent.wheel(graphArea(container), { deltaY: -40, metaKey: true, clientX: pointer.x, clientY: pointer.y });

    const afterMeta = currentView(container);
    expect(afterMeta.k).toBeGreaterThan(opening.k);
    expect(drawingPointAt(afterMeta, pointer).x).toBeCloseTo(drawingPointAt(opening, pointer).x, 6);
    expect(drawingPointAt(afterMeta, pointer).y).toBeCloseTo(drawingPointAt(opening, pointer).y, 6);

    fireEvent.wheel(graphArea(container), { deltaY: -40, ctrlKey: true, clientX: pointer.x, clientY: pointer.y });

    const afterCtrl = currentView(container);
    expect(afterCtrl.k).toBeGreaterThan(afterMeta.k);
    expect(drawingPointAt(afterCtrl, pointer).x).toBeCloseTo(drawingPointAt(opening, pointer).x, 6);
    expect(drawingPointAt(afterCtrl, pointer).y).toBeCloseTo(drawingPointAt(opening, pointer).y, 6);
  });
});

describe("the story labels", () => {
  test("are shown below 46% and gone at 51%, with one label per story by its short title; below 30% boxes are bars", async () => {
    const { container } = render(<Graph nodes={testTree()} />);
    await zoomTo100();
    expect(screen.queryByRole("button", { name: SET_UP_STORY })).toBeNull();

    // 100% / 1.4 / 1.4 is 51%; once more is 36%.
    fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    await waitFor(() => expect(zoomShown()).toBe("71%"));
    fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    await waitFor(() => expect(zoomShown()).toBe("51%"));
    expect(screen.queryByRole("button", { name: SET_UP_STORY })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    await waitFor(() => expect(zoomShown()).toBe("36%"));

    // The test tree holds 13 stories.
    expect(container.querySelectorAll("[data-story-label]")).toHaveLength(13);
    expect(screen.getByRole("button", { name: SET_UP_STORY }).hasAttribute("data-story-label")).toBe(true);
    expect(screen.getByRole("button", { name: PICK_PROGRAM_STORY }).hasAttribute("data-story-label")).toBe(true);

    // At 36% boxes still show their text; at 26% they are bars.
    expect(graphArea(container).hasAttribute("data-far")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    await waitFor(() => expect(zoomShown()).toBe("26%"));
    expect(graphArea(container).hasAttribute("data-far")).toBe(true);
  });

  test("pressing a story's label fits that story's block in the graph area, at no more than 100%", async () => {
    const { container } = render(<Graph nodes={testTree()} />);
    const opening = currentView(container);
    // The block is the story and every node under it; ids in the test tree are folder paths.
    const block = [...container.querySelectorAll<HTMLElement>("[data-node]")]
      .filter((box) => box.dataset.node === "player-picks-program" || box.dataset.node!.startsWith("player-picks-program/"))
      .map((box) => {
        const [, bx, by] = /translate\((-?[\d.]+)px, (-?[\d.]+)px\)/.exec(box.style.transform)!;
        return { x: Number(bx), y: Number(by), w: parseFloat(box.style.width), h: parseFloat(box.style.height) };
      });
    const left = Math.min(...block.map((b) => b.x));
    const top = Math.min(...block.map((b) => b.y));
    const right = Math.max(...block.map((b) => b.x + b.w));
    const bottom = Math.max(...block.map((b) => b.y + b.h));

    fireEvent.click(screen.getByRole("button", { name: PICK_PROGRAM_STORY }));
    await waitFor(() => expect(currentView(container).k).toBeGreaterThan(opening.k * 2));
    // Wait for the move to land: two samples 150ms apart show the same view.
    let last: View | undefined;
    await waitFor(
      () => {
        const now = currentView(container);
        const still = last !== undefined && now.x === last.x && now.y === last.y && now.k === last.k;
        last = now;
        expect(still).toBe(true);
      },
      { interval: 150, timeout: 2000 },
    );

    const view = currentView(container);
    expect(view.k).toBeLessThanOrEqual(1);
    // P18: 64px to spare on each side, 30 more at the top.
    expect(view.x + left * view.k).toBeGreaterThanOrEqual(64 - 1);
    expect(view.y + top * view.k).toBeGreaterThanOrEqual(64 + 30 - 1);
    expect(view.x + right * view.k).toBeLessThanOrEqual(AREA.w - 64 + 1);
    expect(view.y + bottom * view.k).toBeLessThanOrEqual(AREA.h - 64 + 1);
  });
});

describe("the small map", () => {
  test("pressing and dragging in it centres the graph area on that point of the drawing at the same zoom, at once", () => {
    const { container } = render(<Graph nodes={testTree()} />);
    const opening = currentView(container);
    const map = screen.getByLabelText("Map of the tree");

    // The 4276 x 4896 drawing is drawn at 170/4896 so that it is at most 150 x 170.
    fireEvent.pointerDown(map, { pointerId: 2, button: 0, clientX: 75, clientY: 85 });
    expect(currentView(container).k).toBe(opening.k);
    expect(drawingPointAt(currentView(container), MIDDLE).x).toBeCloseTo(2160, 6);
    expect(drawingPointAt(currentView(container), MIDDLE).y).toBeCloseTo(2448, 6);

    fireEvent.pointerMove(map, { pointerId: 2, clientX: 17, clientY: 34 });
    expect(drawingPointAt(currentView(container), MIDDLE).x).toBeCloseTo(489.6, 6);
    expect(drawingPointAt(currentView(container), MIDDLE).y).toBeCloseTo(979.2, 6);
  });
});

describe("the nodes marked as waiting", () => {
  test("are the nodes of open questions an agent asked; a question a person asked is a remark and is left out", async () => {
    const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
    const question = (id: string, nodeId: string, askedBy: Record<string, unknown>) => ({
      id,
      node_id: nodeId,
      node_path: "",
      question: "Which putt counts?",
      state: "open",
      asked_by: askedBy,
      answers: [],
      created_at: "2026-11-12T09:00:00Z",
      version: 1,
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        json({
          questions: [
            question("q1", "node-asked-by-agent", { kind: "agent", run: "r1", role: "grill", model: "m", harness: "h" }),
            question("q2", "node-with-remark", { kind: "human", user_id: "u1" }),
          ],
        }),
      ),
    );

    expect(await loadWaitingNodes("w", "p", "f")).toEqual(["node-asked-by-agent"]);
  });
});
