import type { TreeNode } from "@/api/behaviour";
import { childrenOf, shortTitle } from "./tree";

// Where every box of the graph stands. Pure: sizes come from the data, never from the page.

export type Rect = { x: number; y: number; w: number; h: number };
export type Layout = { boxes: Map<string, Rect>; bounds: Rect; blocks: Map<string, Rect> };

export const BOX_W = 252;
export const LEVEL_GAP = 78;
export const STEP = BOX_W + LEVEL_GAP;
const SHORT_H = 30;
const TALL_H = 46;
const STACK_GAP = 8;
const BLOCK_GAP = 112;
const COLUMN_GAP = 110;
const MAX_COLUMNS = 6;

const boxHeight = (node: TreeNode, isStory: boolean) => (isStory || shortTitle(node).length > 36 ? TALL_H : SHORT_H);

type Tree = { root: TreeNode | undefined; kids: (id: string) => TreeNode[]; stories: TreeNode[] };

function treeOf(nodes: TreeNode[], shown: ReadonlySet<string> | null): Tree {
  const children = childrenOf(nodes);
  const root = (children.get(null) ?? [])[0];
  const kids = (id: string) => (children.get(id) ?? []).filter((child) => !shown || shown.has(child.id));
  return { root, kids, stories: root ? kids(root.id) : [] };
}

/** Where each box, each story's block and the whole drawing stand; `columns` says which story sits in which column. */
export function layoutBlocks(
  nodes: TreeNode[],
  { shown, folded, columns }: { shown: ReadonlySet<string> | null; folded: ReadonlySet<string>; columns: string[][] },
): Layout {
  const { kids, stories } = treeOf(nodes, shown);
  const boxes = new Map<string, Rect>();
  const blocks = new Map<string, Rect>();
  const byId = new Map(nodes.map((node) => [node.id, node]));
  const visible = (id: string) => (folded.has(id) ? [] : kids(id));
  const shift = (id: string, dy: number) => {
    for (const child of visible(id)) {
      boxes.get(child.id)!.y += dy;
      shift(child.id, dy);
    }
  };
  // Places the tree under `id` with its first box at the cursor; returns the lowest edge reached.
  const place = (id: string, depth: number, x0: number, cursor: { y: number }, isStory: boolean) => {
    const node = byId.get(id)!;
    const h = boxHeight(node, isStory);
    const x = x0 + depth * STEP;
    const children = visible(id);
    if (!children.length) {
      boxes.set(id, { x, y: cursor.y, w: BOX_W, h });
      cursor.y += h + STACK_GAP;
      return;
    }
    const start = cursor.y;
    for (const child of children) place(child.id, depth + 1, x0, cursor, false);
    const first = boxes.get(children[0].id)!;
    const last = boxes.get(children[children.length - 1].id)!;
    let y = (first.y + first.h / 2 + last.y + last.h / 2) / 2 - h / 2;
    if (y < start) {
      shift(id, start - y);
      cursor.y += start - y;
      y = start;
    }
    boxes.set(id, { x, y, w: BOX_W, h });
    cursor.y = Math.max(cursor.y, y + h + STACK_GAP);
  };
  const widths = blockWidths(kids, stories);
  let x = 0;
  for (const column of columns) {
    let y = 0;
    let wide = 0;
    for (const id of column) {
      if (!byId.has(id) || !stories.some((story) => story.id === id)) continue;
      const cursor = { y };
      place(id, 0, x, cursor, true);
      const rect = subtreeBounds(id, boxes, visible);
      blocks.set(id, rect);
      y = cursor.y - STACK_GAP + BLOCK_GAP;
      wide = Math.max(wide, widths.get(id)! - LEVEL_GAP);
    }
    x += wide + COLUMN_GAP;
  }
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  for (const box of boxes.values()) {
    x0 = Math.min(x0, box.x);
    y0 = Math.min(y0, box.y);
    x1 = Math.max(x1, box.x + box.w);
    y1 = Math.max(y1, box.y + box.h);
  }
  const bounds = boxes.size ? { x: x0, y: y0, w: x1 - x0, h: y1 - y0 } : { x: 0, y: 0, w: 0, h: 0 };
  return { boxes, bounds, blocks };
}

function subtreeBounds(id: string, boxes: Map<string, Rect>, visible: (id: string) => TreeNode[]): Rect {
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
  const walk = (at: string) => {
    const box = boxes.get(at)!;
    x0 = Math.min(x0, box.x);
    y0 = Math.min(y0, box.y);
    x1 = Math.max(x1, box.x + box.w);
    y1 = Math.max(y1, box.y + box.h);
    for (const child of visible(at)) walk(child.id);
  };
  walk(id);
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

// A block's width is that of all its levels, whether folded or not, so folding never moves a column.
function blockWidths(kids: (id: string) => TreeNode[], stories: TreeNode[]) {
  const depth = (id: string): number => {
    const below = kids(id);
    return below.length ? 1 + Math.max(...below.map((child) => depth(child.id))) : 0;
  };
  return new Map(stories.map((story) => [story.id, (depth(story.id) + 1) * STEP]));
}

/** Which story sits in which column: 1 to 6 columns filled in story order; the count whose drawing is closest in shape to `area` wins. */
export function planColumns(nodes: TreeNode[], shown: ReadonlySet<string> | null, area: { w: number; h: number }): string[][] {
  const { kids, stories } = treeOf(nodes, shown);
  if (!stories.length) return [];
  const leaves = (id: string): number => {
    const below = kids(id);
    return below.length ? below.reduce((sum, child) => sum + leaves(child.id), 0) : 1;
  };
  const heights = stories.map((story) => leaves(story.id) * 44 + 70);
  const widthOf = blockWidths(kids, stories);
  const widths = stories.map((story) => widthOf.get(story.id)!);
  const total = heights.reduce((a, b) => a + b, 0);
  const want = area.w / Math.max(area.h, 1);
  let best: { score: number; columns: string[][] } | null = null;
  for (let count = 1; count <= MAX_COLUMNS; count++) {
    const target = total / count;
    const cols: number[][] = [[]];
    let height = 0;
    stories.forEach((_, i) => {
      const current = cols[cols.length - 1];
      if (current.length && height + heights[i] / 2 > target && cols.length < count) {
        cols.push([]);
        height = 0;
      }
      cols[cols.length - 1].push(i);
      height += heights[i];
    });
    const w = cols.reduce((sum, col) => sum + Math.max(...col.map((i) => widths[i])) + COLUMN_GAP, 0);
    const h = Math.max(...cols.map((col) => col.reduce((sum, i) => sum + heights[i], 0)));
    const score = Math.abs(Math.log(w / h / want));
    if (!best || score < best.score) best = { score, columns: cols.map((col) => col.map((i) => stories[i].id)) };
  }
  return best!.columns;
}

/** The path of the line from `parent` to `child`: out of the right edge, to 26 before the child, turning with rounded corners. */
export function elbow(parent: Rect, child: Rect): string {
  const x1 = parent.x + parent.w;
  const y1 = parent.y + parent.h / 2;
  const x2 = child.x;
  const y2 = child.y + child.h / 2;
  const mx = x2 - 26;
  if (Math.abs(y1 - y2) < 1) return `M${x1},${y1}H${x2}`;
  const r = Math.min(8, Math.abs(y2 - y1) / 2);
  const s = y2 > y1 ? 1 : -1;
  return `M${x1},${y1}H${mx - r}Q${mx},${y1} ${mx},${y1 + s * r}V${y2 - s * r}Q${mx},${y2} ${mx + r},${y2}H${x2}`;
}
