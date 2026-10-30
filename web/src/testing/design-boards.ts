// US-058 and US-112: the design viewer reads a design's head board by board,
// never its whole body at once:
//
// - GET .../docs/<doc>/html?view=outline answers the head's `outline`: its
//   pages in order, each page as its own JSON with its boards in order under
//   `children`, every board as its own JSON (name, box, fills, layout) without
//   its `children`. A board of the outline is the placeholder that the
//   renderer draws until the board is loaded. `boards` has the chunk of each
//   board; `components` and `tokens` are as in the whole read.
// - GET .../docs/<doc>/boards/<board> answers one board of the head:
//   `{"doc_id", "page_id", "board_id", "version_id", "tags", "node"}`, where
//   `node` is the board and its subtree as JSON, as the whole read's `pages`
//   hold it.
//
// The fakes of the API in the design tests serve these reads from the whole
// heads they hold, with this module.

export type JsonNode = Record<string, unknown> & { id?: unknown; children?: unknown };

/** A node without its children. */
export function own(node: JsonNode): JsonNode {
  const rest = { ...node };
  delete rest.children;
  return rest;
}

function childrenOf(node: JsonNode): JsonNode[] {
  return Array.isArray(node.children) ? (node.children as JsonNode[]) : [];
}

/** The outline of `pages`: each page with its boards as placeholders, without their content. */
export function outlineOf(pages: readonly JsonNode[]): JsonNode[] {
  return pages.map((page) => ({ ...page, children: childrenOf(page).map(own) }));
}

/** What GET .../docs/<doc>/boards/<board> answers, or null when `head` has no such board. */
export function boardOf(head: JsonNode, boardId: string): JsonNode | null {
  const pages = Array.isArray(head.pages) ? (head.pages as JsonNode[]) : [];
  for (const page of pages) {
    const board = childrenOf(page).find((candidate) => candidate.id === boardId);
    if (board) {
      return {
        doc_id: head.doc_id,
        page_id: page.id,
        board_id: boardId,
        version_id: head.id,
        tags: `<Board id="${boardId}"/>`,
        node: board,
      };
    }
  }
  return null;
}

/** A name for the content of `board` that changes when the board does, as a chunk's hash does. */
export function chunkOf(board: JsonNode): string {
  let hash = 0x811c9dc5;
  for (const char of JSON.stringify(board)) hash = Math.imul(hash ^ char.charCodeAt(0), 0x01000193) >>> 0;
  return hash.toString(16).padStart(8, "0");
}

/** What GET .../docs/<doc>/html?view=outline answers for a design whose head is `head` (pages, components, tokens). */
export function outlineHead(head: JsonNode): JsonNode {
  const pages = Array.isArray(head.pages) ? (head.pages as JsonNode[]) : [];
  return {
    doc_id: head.doc_id,
    html: "",
    outline: outlineOf(pages),
    boards: pages.flatMap((page) => childrenOf(page).map((board) => ({ id: board.id, chunk: chunkOf(board) }))),
    components: head.components ?? [],
    tokens: head.tokens ?? null,
  };
}

/**
 * Answers the board-by-board reads of a fake API from the heads that `headOf`
 * finds by doc ID: the outline of the head, and one board of the head. `null`
 * when the request is neither, so that the fake answers it as before.
 */
export function answerDesignRead(
  api: string,
  method: string,
  url: URL,
  headOf: (docId: string) => JsonNode | null | undefined,
): { status: number; body: unknown } | null {
  if (method !== "GET") return null;
  const path = url.pathname;
  const escaped = api.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const html = path.match(new RegExp(`^${escaped}/docs/([^/]+)/html$`));
  if (html && url.searchParams.get("view") === "outline") {
    const head = headOf(html[1]);
    return head ? { status: 200, body: outlineHead({ ...head, doc_id: html[1] }) } : null;
  }
  const board = path.match(new RegExp(`^${api.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/docs/([^/]+)/boards/([^/]+)$`));
  if (board) {
    const [, docId, boardId] = board;
    const head = headOf(docId);
    if (!head) return { status: 404, body: { error: "not_found" } };
    const found = boardOf({ ...head, doc_id: docId }, boardId);
    return found ? { status: 200, body: found } : { status: 404, body: { error: "unknown_board" } };
  }
  return null;
}

/** The IDs of the boards of `page` (a page as the renderer was shown it). */
export function boardIdsOf(page: unknown): string[] {
  if (!page || typeof page !== "object") return [];
  return childrenOf(page as JsonNode).map((board) => String(board.id));
}
