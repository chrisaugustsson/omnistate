import type { TreeNode } from "@/api/behaviour";

/** The chosen node's details; for now only its full title. */
export function Details({ node }: { node: TreeNode | null }) {
  return (
    <section aria-label="Details" className="min-h-0 overflow-y-auto border-l">
      {node && <h2 className="px-5 pt-5 text-[15px] font-semibold leading-snug">{node.title}</h2>}
    </section>
  );
}
