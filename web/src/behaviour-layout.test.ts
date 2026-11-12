// US-223: the graph's layout and the view that fits it, as plain functions.
// layoutBlocks places every node but the root as a box, one block per story;
// planColumns chooses which story sits in which column; fit is the view that
// shows a whole drawing in the graph area. The test tree is the PRD's tree of
// 323 nodes and 13 stories; the small story's places are worked out by hand
// from the PRD's numbers.
import { describe, expect, test } from "vitest";
import type { TreeNode } from "./api/behaviour";
import { fit } from "./components/behaviour/camera";
import { layoutBlocks, planColumns } from "./components/behaviour/layout";
import { testTree } from "./testing/behaviour-test-tree";

const NO_FOLDS: ReadonlySet<string> = new Set();

function node(id: string, parentId: string | null, title: string, position = 0): TreeNode {
  return {
    id,
    parent_id: parentId,
    path: id,
    slug: null,
    position,
    level: 0,
    title,
    summary: "",
    status: "answered",
    version: 1,
    open_questions: 0,
    default_answers: 0,
    needs_pass: false,
    same_as: null,
    handoff: null,
  };
}

describe("planColumns on the test tree", () => {
  test("gives 2 columns for a tall narrow area, 3 for 748 x 844 and 6 for a wide low one", () => {
    const nodes = testTree();

    expect(planColumns(nodes, null, { w: 300, h: 900 })).toHaveLength(2);
    expect(planColumns(nodes, null, { w: 748, h: 844 })).toHaveLength(3);
    expect(planColumns(nodes, null, { w: 1800, h: 600 })).toHaveLength(6);
  });

  test("fills the columns with the 13 stories in story order", () => {
    const nodes = testTree();
    const stories = nodes.filter((n) => n.parent_id === "feature").map((n) => n.id);

    expect(planColumns(nodes, null, { w: 748, h: 844 }).flat()).toEqual(stories);
  });
});

describe("layoutBlocks on the test tree in a 748 x 844 graph area", () => {
  const nodes = testTree();
  const layout = layoutBlocks(nodes, {
    shown: null,
    folded: NO_FOLDS,
    columns: planColumns(nodes, null, { w: 748, h: 844 }),
  });

  test("draws a box for each of the 322 nodes but the feature's root, in a drawing of 4276 x 4896", () => {
    expect(layout.boxes.size).toBe(322);
    expect(layout.boxes.has("feature")).toBe(false);
    expect(Math.abs(layout.bounds.w - 4276)).toBeLessThanOrEqual(1);
    expect(Math.abs(layout.bounds.h - 4896)).toBeLessThanOrEqual(1);
  });

  test("stands every child 330 to the right of its parent", () => {
    const misplaced = nodes
      .filter((n) => n.parent_id && n.parent_id !== "feature")
      .filter((n) => layout.boxes.get(n.id)!.x - layout.boxes.get(n.parent_id!)!.x !== 330)
      .map((n) => n.id);

    expect(misplaced).toEqual([]);
  });

  test("on opening the whole drawing fits inside the graph area at a zoom between 13% and 15%", () => {
    const view = fit(layout.bounds, { w: 748, h: 844 });

    expect(view.k).toBeGreaterThanOrEqual(0.13);
    expect(view.k).toBeLessThanOrEqual(0.15);
    expect(view.x + layout.bounds.x * view.k).toBeGreaterThanOrEqual(0);
    expect(view.y + layout.bounds.y * view.k).toBeGreaterThanOrEqual(0);
    expect(view.x + (layout.bounds.x + layout.bounds.w) * view.k).toBeLessThanOrEqual(748);
    expect(view.y + (layout.bounds.y + layout.bounds.h) * view.k).toBeLessThanOrEqual(844);
  });
});

test("layoutBlocks sizes a story's boxes by short title, stacks them 8 apart and centres a parent on its first and last child", () => {
  // A story holding a node whose title has 36 characters, one with 37, and one
  // with a node under it.
  const nodes = [
    node("feature", null, "Disc golf putting trainer"),
    node("story", "feature", "As a player, I want to log a practice, so that it counts"),
    node("at-36", "story", "Player sees the make rate per stance"),
    node("at-37", "story", "Player sees the make rate per stance.", 1),
    node("parent", "story", "Player logs a round", 2),
    node("child", "parent", "Player saves it"),
  ];

  const { boxes } = layoutBlocks(nodes, { shown: null, folded: NO_FOLDS, columns: [["story"]] });

  expect(boxes.get("story")).toEqual({ x: 0, y: 38, w: 252, h: 46 });
  expect(boxes.get("at-36")).toEqual({ x: 330, y: 0, w: 252, h: 30 });
  expect(boxes.get("at-37")).toEqual({ x: 330, y: 38, w: 252, h: 46 });
  expect(boxes.get("parent")).toEqual({ x: 330, y: 92, w: 252, h: 30 });
  expect(boxes.get("child")).toEqual({ x: 660, y: 92, w: 252, h: 30 });
});
