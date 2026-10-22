// US-058: the design viewer and the change set review read a design version
// board by board, never its whole body at once:
//
// - GET .../versions/<version>?view=outline answers the version without
//   `pages` and `tags`, and with `outline`: its pages in order, each page as
//   its own JSON with its boards in order under `children`, every board as its
//   own JSON (name, box, fills, layout) without its `children`. A board of the
//   outline is the placeholder that the renderer draws until the board is
//   loaded. The version's `manifest`, `components` and `tokens` are as in the
//   whole read; its `nodes` list nothing below a board.
// - GET .../docs/<doc>/boards/<board>?version=<version> answers one board of
//   that version: `{"doc_id", "page_id", "board_id", "version_id", "tags",
//   "node"}`, where `node` is the board and its subtree as JSON, as the whole
//   read's `pages` hold it.
//
// The fakes of the API in the design tests serve both reads from the whole
// versions they hold, with this module.

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

/** What GET .../versions/<id>?view=outline answers for the design version `version`. */
export function outlineVersion(version: JsonNode): JsonNode {
  const rest = { ...version };
  delete rest.pages;
  delete rest.tags;
  const pages = Array.isArray(version.pages) ? (version.pages as JsonNode[]) : [];
  return { ...rest, nodes: [], outline: outlineOf(pages) };
}

/** What GET .../docs/<doc>/boards/<board>?version=<id> answers, or null when `version` has no such board. */
export function boardOf(version: JsonNode, boardId: string): JsonNode | null {
  const pages = Array.isArray(version.pages) ? (version.pages as JsonNode[]) : [];
  for (const page of pages) {
    const board = childrenOf(page).find((candidate) => candidate.id === boardId);
    if (board) {
      return {
        doc_id: version.doc_id,
        page_id: page.id,
        board_id: boardId,
        version_id: version.id,
        tags: `<Board id="${boardId}"/>`,
        node: board,
      };
    }
  }
  return null;
}

/**
 * Answers the board-by-board reads of a fake API from the whole design
 * versions that `versionOf` finds by ID: the outline of a version, and one
 * board of a version. `null` when the request is neither, so that the fake
 * answers it as before.
 */
export function answerDesignRead(
  api: string,
  method: string,
  url: URL,
  versionOf: (id: string) => JsonNode | null | undefined,
): { status: number; body: unknown } | null {
  if (method !== "GET") return null;
  const path = url.pathname;
  const versionPath = path.startsWith(`${api}/versions/`) ? path.slice(`${api}/versions/`.length) : null;
  if (versionPath && !versionPath.includes("/") && url.searchParams.get("view") === "outline") {
    const version = versionOf(versionPath);
    if (!version || !Array.isArray(version.pages)) return null;
    return { status: 200, body: { version: outlineVersion(version) } };
  }
  const board = path.match(new RegExp(`^${api.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/docs/([^/]+)/boards/([^/]+)$`));
  if (board) {
    const [, docId, boardId] = board;
    const versionId = url.searchParams.get("version");
    const version = versionId ? versionOf(versionId) : null;
    if (!version || version.doc_id !== docId) return { status: 404, body: { error: "not_found" } };
    const found = boardOf(version, boardId);
    return found ? { status: 200, body: found } : { status: 404, body: { error: "unknown_board" } };
  }
  return null;
}

/** The IDs of the boards of `page` (a page as the renderer was shown it). */
export function boardIdsOf(page: unknown): string[] {
  if (!page || typeof page !== "object") return [];
  return childrenOf(page as JsonNode).map((board) => String(board.id));
}
