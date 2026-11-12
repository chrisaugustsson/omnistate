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
