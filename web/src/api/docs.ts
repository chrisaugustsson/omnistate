import { callApi } from "./client";
import {
  operations,
  type ChangesetBody,
  type ClaimBody,
  type CommentBody,
  type DocBody,
  type DraftWithHtml,
  type VersionBody,
  type VersionItem,
} from "./generated";

const base = (workspaceId: string, projectId: string) => `/api/v1/w/${workspaceId}/projects/${projectId}`;

const doc = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}"];
const versions = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/versions"];
const version = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}"];
const board = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/boards/{board_id}"];
const html = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/html"];
const draft = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}"];
const changesets = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/changesets"];
const comments = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/comments"];
const comment = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/comments"];
const headComment = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/comments"];

/** A doc with its drafts and live claims. */
export async function loadDoc(workspaceId: string, projectId: string, docId: string): Promise<DocBody> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/docs/${docId}`, {
    response: doc.responses["200"],
  });
  return loaded.doc;
}

/** Every saved version of a doc, oldest first. */
export async function loadVersions(workspaceId: string, projectId: string, docId: string): Promise<VersionItem[]> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/docs/${docId}/versions`, {
    response: versions.responses["200"],
  });
  return loaded.versions;
}

/** A saved version by its ID; a design version holds its pages, components and tokens. */
export async function loadVersion(workspaceId: string, projectId: string, versionId: string): Promise<VersionBody> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/versions/${versionId}`, {
    response: version.responses["200"],
  });
  return loaded.version;
}

/**
 * A saved design version as its outline: its pages with their boards as
 * placeholders (`outline`), the components and the tokens of the design system
 * it recorded, and no board content. The boards come one by one from
 * `loadBoard`, so a big design is never read whole.
 */
export async function loadVersionOutline(workspaceId: string, projectId: string, versionId: string): Promise<VersionBody> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/versions/${versionId}?view=outline`, {
    response: version.responses["200"],
  });
  return loaded.version;
}

/** One board of a version of a design doc, with everything below it, as JSON. */
export async function loadBoard(
  workspaceId: string,
  projectId: string,
  docId: string,
  versionId: string,
  boardId: string,
): Promise<Record<string, unknown>> {
  const loaded = await callApi(
    "GET",
    `${base(workspaceId, projectId)}/docs/${docId}/boards/${boardId}?version=${encodeURIComponent(versionId)}`,
    { response: board.responses["200"] },
  );
  return loaded.node;
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

/** The HTML of a version of the doc, or of its mainline version; empty when there is none. */
export async function loadVersionHtml(
  workspaceId: string,
  projectId: string,
  docId: string,
  versionId?: string,
): Promise<string> {
  const query = versionId ? `?version=${encodeURIComponent(versionId)}` : "";
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/docs/${docId}/html${query}`, {
    response: html.responses["200"],
  });
  return loaded.html;
}

/**
 * A change set's draft of the doc, as the agents have it now: the HTML of an
 * HTML doc, the pages, components and tokens of a design.
 */
export async function loadDraft(
  workspaceId: string,
  projectId: string,
  changesetId: string,
  docId: string,
): Promise<DraftWithHtml> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/changesets/${changesetId}/docs/${docId}`, {
    response: draft.responses["200"],
  });
  return loaded.draft;
}

/** The HTML of a change set's draft of the doc, as the agents have it now. */
export async function loadDraftHtml(
  workspaceId: string,
  projectId: string,
  changesetId: string,
  docId: string,
): Promise<string> {
  return (await loadDraft(workspaceId, projectId, changesetId, docId)).html;
}

/** The change sets of a project. */
export async function loadChangesets(workspaceId: string, projectId: string): Promise<ChangesetBody[]> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/changesets`, {
    response: changesets.responses["200"],
  });
  return loaded.changesets;
}

/** The comments on every version of a doc. */
export async function loadDocComments(workspaceId: string, projectId: string, docId: string): Promise<CommentBody[]> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/comments?doc=${encodeURIComponent(docId)}`, {
    response: comments.responses["200"],
  });
  return loaded.comments;
}

/** Comments on a block of a saved version. */
export async function createComment(
  workspaceId: string,
  projectId: string,
  versionId: string,
  fields: { node: string; text: string },
): Promise<CommentBody> {
  const created = await callApi("POST", `${base(workspaceId, projectId)}/versions/${versionId}/comments`, {
    body: fields,
    response: comment.responses["201"],
  });
  return created.comment;
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

/** The holder of a claim as `<run>/<role>`. */
export function holderOf(claim: ClaimBody): string {
  return `${String(claim.agent.run)}/${String(claim.agent.role)}`;
}
