import { callApi } from "./client";
import {
  operations,
  type ChangesetBody,
  type ClaimBody,
  type CommentBody,
  type DocBody,
  type VersionItem,
} from "./generated";

const base = (workspaceId: string, projectId: string) => `/api/v1/w/${workspaceId}/projects/${projectId}`;

const doc = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}"];
const versions = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/versions"];
const html = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs/{doc_id}/html"];
const draft = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/docs/{doc_id}"];
const changesets = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/changesets"];
const comments = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/comments"];
const comment = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/versions/{version_id}/comments"];

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

/** The HTML of a change set's draft of the doc, as the agents have it now. */
export async function loadDraftHtml(
  workspaceId: string,
  projectId: string,
  changesetId: string,
  docId: string,
): Promise<string> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/changesets/${changesetId}/docs/${docId}`, {
    response: draft.responses["200"],
  });
  return loaded.draft.html;
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

/** The holder of a claim as `<run>/<role>`. */
export function holderOf(claim: ClaimBody): string {
  return `${String(claim.agent.run)}/${String(claim.agent.role)}`;
}
