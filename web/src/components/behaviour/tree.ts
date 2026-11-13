import type { TreeNode } from "@/api/behaviour";

const STORY = /^As an? (.+?), I want (?:to )?(.+?),? so that .+$/s;

/** A story title `As a <role>, I want [to] <flow> so that <purpose>` as `<Role>: <flow>`; any other title whole. */
export function shortTitle(node: Pick<TreeNode, "title">): string {
  const match = STORY.exec(node.title.trim());
  if (!match) return node.title;
  return `${match[1].charAt(0).toUpperCase()}${match[1].slice(1)}: ${match[2]}`;
}

/** The children of each node by its id, in the order of `nodes`; the root is under `null`. */
export function childrenOf(nodes: TreeNode[]): Map<string | null, TreeNode[]> {
  const children = new Map<string | null, TreeNode[]>();
  for (const node of nodes) {
    const key = node.parent_id ?? null;
    const siblings = children.get(key);
    if (siblings) siblings.push(node);
    else children.set(key, [node]);
  }
  return children;
}

/** The ids above `id`, the nearest parent first and the root last; empty for the root or an unknown id. */
export function ancestorsOf(nodes: TreeNode[], id: string): string[] {
  const parents = new Map(nodes.map((node) => [node.id, node.parent_id ?? null]));
  const above: string[] = [];
  for (let at = parents.get(id) ?? null; at !== null && !above.includes(at); at = parents.get(at) ?? null) above.push(at);
  return above;
}

/** The ids of everything under `id`, in tree order. */
export function descendantsOf(nodes: TreeNode[], id: string): string[] {
  const children = childrenOf(nodes);
  const below: string[] = [];
  const walk = (parent: string) => {
    for (const child of children.get(parent) ?? []) {
      below.push(child.id);
      walk(child.id);
    }
  };
  walk(id);
  return below;
}

export type Filter = "all" | "open" | "default" | "needs_pass";

/** Whether `node` is one a filter picks out. */
export function matches(node: TreeNode, filter: Filter): boolean {
  if (filter === "open") return node.status === "open";
  if (filter === "default") return node.default_answers > 0;
  if (filter === "needs_pass") return node.needs_pass;
  return true;
}

/** The nodes `filter` picks out and what they show with: those, every node above one of them, the chosen node and what lies above it. */
export function shownUnder(nodes: TreeNode[], filter: Filter, chosen: string | null): { matching: Set<string>; shown: Set<string> } {
  const matching = new Set(nodes.filter((node) => matches(node, filter)).map((node) => node.id));
  const shown = new Set(matching);
  const parents = new Map(nodes.map((node) => [node.id, node.parent_id ?? null]));
  const addAbove = (id: string) => {
    for (let at = parents.get(id) ?? null; at !== null && !shown.has(at); at = parents.get(at) ?? null) shown.add(at);
  };
  for (const id of matching) addAbove(id);
  if (chosen && parents.has(chosen)) {
    shown.add(chosen);
    addAbove(chosen);
  }
  const root = nodes.find((node) => !node.parent_id);
  if (root) shown.add(root.id);
  return { matching, shown };
}

/** The words of a search query: split on whitespace, none for a blank query. */
export function words(query: string): string[] {
  return query.trim().split(/\s+/).filter(Boolean);
}

/** The ids, in tree order, of the nodes whose title, summary and slug together hold every word of `query`, case ignored; only among `among` when given. The root is never found. */
export function find(nodes: TreeNode[], query: string, among?: ReadonlySet<string> | null): string[] {
  const wanted = words(query).map((word) => word.toLowerCase());
  if (wanted.length === 0) return [];
  return nodes
    .filter((node) => {
      if (!node.parent_id || (among && !among.has(node.id))) return false;
      const text = `${node.title} ${node.summary} ${node.slug ?? ""}`.toLowerCase();
      return wanted.every((word) => text.includes(word));
    })
    .map((node) => node.id);
}
