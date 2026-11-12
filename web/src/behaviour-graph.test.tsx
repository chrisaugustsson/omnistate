// @vitest-environment jsdom
// US-224: moving around the Behaviour screen's graph. The graph of the PRD's
// test tree is rendered in a graph area of 748 x 844 (the one a 1440 x 900
// window gives) and the view is read back from the drawing's transform:
// `translate(Xpx, Ypx) scale(K)` puts the drawing's point (px, py) at
// (X + px * K, Y + py * K) in the graph area.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { TreeNode } from "./api/behaviour";
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

// US-225: what a box shows. Its status (a signal ring before the title of an open
// node), its marks (a badge of waiting questions, a diamond for answers given
// by default, a pointer mark, ↻ for a node waiting for the next grill pass) and,
// below 30% zoom, the colour of its bar: signal for a waiting question, ink for
// a node that needs another pass, ink when it has both (P12).

const SEARCHES = "player-adds-friend/player-searches-for-player";
const SHARES_LINK = "player-adds-friend/player-shares-invite-link";
const NO_PROGRAM = "player-picks-program/new-player-has-no-program";
const OPENS_LIST = "player-picks-program/player-opens-program-list";

/** The test tree with `changes` applied to some of its nodes. */
function treeWith(changes: Record<string, Partial<TreeNode>>): TreeNode[] {
  return testTree().map((node) => ({ ...node, ...changes[node.id] }));
}

function box(container: HTMLElement, id: string) {
  return container.querySelector<HTMLElement>(`[data-node="${id}"]`)!;
}

/** The colours a box's bar is given below 30% zoom, as its classes for that zoom name them. */
function barColours(container: HTMLElement, id: string) {
  return [...box(container, id).classList].filter((name) => /^group-data-\[far\]:bg-/.test(name));
}

describe("a box's status and marks", () => {
  test("below 30% the bar of a node with a waiting question is signal, of a node needing another pass ink, and ink when it has both", () => {
    const nodes = treeWith({
      [SEARCHES]: { status: "open", needs_pass: true },
      [NO_PROGRAM]: { status: "open" },
      [SHARES_LINK]: { status: "open", needs_pass: true },
    });
    const waiting = new Map([
      [SEARCHES, 1],
      [NO_PROGRAM, 1],
    ]);
    const { container } = render(<Graph nodes={nodes} waiting={waiting} />);
    // The whole test tree is fitted at 14%, so the boxes are bars.
    expect(graphArea(container).hasAttribute("data-far")).toBe(true);

    expect(barColours(container, NO_PROGRAM)).toEqual(["group-data-[far]:bg-signal"]);
    expect(barColours(container, SHARES_LINK)).toEqual(["group-data-[far]:bg-foreground"]);
    expect(barColours(container, SEARCHES)).toEqual(["group-data-[far]:bg-foreground"]);
  });

  test("a node needing another pass shows ↻ and has no ring unless a question still waits on it", () => {
    const nodes = treeWith({
      [SEARCHES]: { status: "open", needs_pass: true },
      [SHARES_LINK]: { status: "open", needs_pass: true },
      [NO_PROGRAM]: { status: "open" },
    });
    const { container } = render(<Graph nodes={nodes} waiting={new Map([[SEARCHES, 1]])} />);
    const ring = (id: string) => box(container, id).querySelector("[data-open-ring]");
    const passMark = (id: string) => box(container, id).querySelector('[data-mark="pass"]')?.textContent;

    expect(passMark(SHARES_LINK)).toBe("↻");
    expect(ring(SHARES_LINK)).toBeNull();
    expect(passMark(SEARCHES)).toBe("↻");
    expect(ring(SEARCHES)).not.toBeNull();
    expect(passMark(NO_PROGRAM)).toBeUndefined();
    expect(ring(NO_PROGRAM)).not.toBeNull();
    expect(ring(OPENS_LIST)).toBeNull();
    expect(box(container, SEARCHES).querySelector('[data-mark="waiting"]')?.textContent).toBe("1");
    expect(box(container, "player-adds-friend").querySelector('[data-mark="waiting"]')).toBeNull();
  });

  test("a pointer shows ↪ or ⇢ with the target's short title after Same as or Hands off to as its tooltip", () => {
    const nodes = treeWith({
      [SEARCHES]: { same_as: { id: "player-picks-program", path: "player-picks-program" } },
      [SHARES_LINK]: { handoff: { id: NO_PROGRAM, path: NO_PROGRAM } },
    });
    const { container } = render(<Graph nodes={nodes} />);
    const pointer = (id: string) => box(container, id).querySelector<HTMLElement>('[data-mark="pointer"]')!;

    expect(pointer(SEARCHES).textContent).toBe("↪");
    expect(pointer(SEARCHES).title).toBe(`Same as: ${PICK_PROGRAM_STORY}`);
    expect(pointer(SHARES_LINK).textContent).toBe("⇢");
    expect(pointer(SHARES_LINK).title).toBe("Hands off to: Player has no active program");
  });

  test("a node with an answer given by default shows a diamond and a node with none does not", () => {
    const nodes = treeWith({ [SEARCHES]: { default_answers: 1 } });
    const { container } = render(<Graph nodes={nodes} />);

    expect(box(container, SEARCHES).querySelector('[data-mark="default"]')).not.toBeNull();
    expect(box(container, SHARES_LINK).querySelector('[data-mark="default"]')).toBeNull();
  });
});

// US-232: a box with nodes under it carries a pill at its right edge, "−" while
// they are shown and "+N" for the N nodes a fold hides, with a signal dot when
// one of the hidden nodes has a waiting question. Under a filter only the nodes
// it shows count, and a box with none of its children shown has no pill.
describe("the fold pills", () => {
  const STORY = "player-adds-friend";
  const SEARCHES = "player-adds-friend/player-searches-for-player";
  const SHARES_LINK = "player-adds-friend/player-shares-invite-link";
  const NO_PLAYER = `${SEARCHES}/no-player-found`;
  const pill = (container: HTMLElement, id: string) => container.querySelector(`[data-node="${id}"] [data-fold]`);
  const dot = (container: HTMLElement, id: string) => container.querySelector(`[data-node="${id}"] [data-fold] [data-fold-dot]`);

  test("a folded box reads +N for every node under it, with a dot only when a node under it has a waiting question", () => {
    const waiting = new Map([[SEARCHES, 1]]);
    const { container, rerender } = render(<Graph nodes={testTree()} waiting={waiting} />);
    expect(pill(container, STORY)?.textContent).toBe("−");
    expect(pill(container, NO_PLAYER)).toBeNull();

    rerender(<Graph nodes={testTree()} waiting={waiting} folded={new Set([STORY])} />);
    expect(pill(container, STORY)?.textContent).toBe("+32");
    expect(dot(container, STORY)).not.toBeNull();
    expect(container.querySelectorAll("[data-node]")).toHaveLength(322 - 32);

    // The question waits on the searches box itself, not on a node its fold hides.
    rerender(<Graph nodes={testTree()} waiting={waiting} folded={new Set([SEARCHES])} />);
    expect(pill(container, SEARCHES)?.textContent).toBe("+7");
    expect(dot(container, SEARCHES)).toBeNull();
    expect(pill(container, STORY)?.textContent).toBe("−");
  });

  test("under a filter a fold counts only the nodes it shows, and a box with none of its children shown has no pill", () => {
    const shown = new Set(["feature", STORY, SEARCHES, NO_PLAYER, `${SEARCHES}/request-sent`, SHARES_LINK]);
    const { container, rerender } = render(<Graph nodes={testTree()} shown={shown} />);
    expect(pill(container, SEARCHES)?.textContent).toBe("−");
    expect(pill(container, SHARES_LINK)).toBeNull();

    rerender(<Graph nodes={testTree()} shown={shown} folded={new Set([STORY])} />);
    expect(pill(container, STORY)?.textContent).toBe("+4");
  });
});
