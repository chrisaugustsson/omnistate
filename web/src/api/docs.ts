import { callApi } from "./client";
import { operations, type CommentBody, type DocBody } from "./generated";

const base = (workspaceId: string, projectId: string) => `/api/v1/w/${workspaceId}/projects/${projectId}`;

const doc = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}"];
const board = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/boards/{board_id}"];
const html = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/html"];
const comments = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/comments"];
const headComment = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/comments"];
const reply = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/comments/{comment_id}/replies"];

/** A doc. */
export async function loadDoc(workspaceId: string, projectId: string, docId: string): Promise<DocBody> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/docs/${docId}`, {
    response: doc.responses["200"],
  });
  return loaded.doc;
}

/** The HTML of the doc's head, as the agents have written it now; empty for a doc with no content yet. */
export async function loadHeadHtml(workspaceId: string, projectId: string, docId: string): Promise<string> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/docs/${docId}/html`, {
    response: html.responses["200"],
  });
  return loaded.html;
}

/**
 * A design's head as its outline: its pages with their boards as placeholders
 * (`outline`), the chunk of each board (`boards`, which changes with the
 * board), and the components and tokens of the design system head. The boards
 * come one by one from `loadHeadBoard`.
 */
export async function loadHeadOutline(workspaceId: string, projectId: string, docId: string) {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/docs/${docId}/html?view=outline`, {
    response: html.responses["200"],
  });
  return {
    pages: loaded.outline ?? [],
    boards: loaded.boards ?? [],
    components: loaded.components ?? [],
    tokens: loaded.tokens ?? null,
  };
}

/**
 * Small pictures of `boards` of a design's head, as Base64 PNGs by board ID. A
 * board that has none is left out.
 */
export async function loadHeadPreviews(
  workspaceId: string,
  projectId: string,
  docId: string,
  boards: string[],
): Promise<Record<string, string>> {
  const loaded = await callApi(
    "GET",
    `${base(workspaceId, projectId)}/docs/${docId}/html?view=previews&boards=${boards.map(encodeURIComponent).join(",")}`,
    { response: html.responses["200"] },
  );
  return loaded.previews ?? {};
}

/** One board of a design's head, with everything below it, as JSON. */
export async function loadHeadBoard(
  workspaceId: string,
  projectId: string,
  docId: string,
  boardId: string,
): Promise<Record<string, unknown>> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/docs/${docId}/boards/${boardId}`, {
    response: board.responses["200"],
  });
  return loaded.node;
}

/** The comments on a doc, those whose element an agent removed too. */
export async function loadDocComments(workspaceId: string, projectId: string, docId: string): Promise<CommentBody[]> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/comments?doc=${encodeURIComponent(docId)}`, {
    response: comments.responses["200"],
  });
  return loaded.comments;
}

/** Comments on a node of the doc's head. */
export async function createDocComment(
  workspaceId: string,
  projectId: string,
  docId: string,
  fields: { node: string; text: string },
): Promise<CommentBody> {
  const created = await callApi("POST", `${base(workspaceId, projectId)}/docs/${docId}/comments`, {
    body: fields,
    response: headComment.responses["201"],
  });
  return created.comment;
}

/** Replies to a comment; the comment's replies, the new one last, come back with it. */
export async function replyToComment(
  workspaceId: string,
  projectId: string,
  commentId: string,
  text: string,
): Promise<CommentBody> {
  const replied = await callApi("POST", `${base(workspaceId, projectId)}/comments/${commentId}/replies`, {
    body: { text },
    response: reply.responses["201"],
  });
  return replied.comment;
}

/** Deletes a comment, with its replies; only the human can. */
export async function deleteComment(workspaceId: string, projectId: string, commentId: string): Promise<void> {
  await callApi("DELETE", `${base(workspaceId, projectId)}/comments/${commentId}`);
}
