import { callApi } from "./client";
import { operations, type ChangesetBody, type RecordedConflict, type ReviewResponse } from "./generated";

const base = (workspaceId: string, projectId: string, changesetId: string) =>
  `/api/v1/w/${workspaceId}/projects/${projectId}/changesets/${changesetId}`;

const review = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/review"];
const conflicts = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/conflicts"];
const approve = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/approve"];
const reject = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/changesets/{changeset_id}/reject"];

/** A change set with the docs it changes, the node changes of each, and its linked decisions. */
export function loadReview(workspaceId: string, projectId: string, changesetId: string): Promise<ReviewResponse> {
  return callApi("GET", `${base(workspaceId, projectId, changesetId)}/review`, {
    response: review.responses["200"],
  });
}

/** The conflicts that the change set's rebases recorded, resolved or not. */
export async function loadConflicts(
  workspaceId: string,
  projectId: string,
  changesetId: string,
): Promise<RecordedConflict[]> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId, changesetId)}/conflicts`, {
    response: conflicts.responses["200"],
  });
  return loaded.conflicts;
}

/** Approves the whole change set, which the server then merges. */
export async function approveChangeset(
  workspaceId: string,
  projectId: string,
  changesetId: string,
): Promise<ChangesetBody> {
  const approved = await callApi("POST", `${base(workspaceId, projectId, changesetId)}/approve`, {
    body: {},
    response: approve.responses["200"],
  });
  return approved.changeset;
}

/** Rejects the whole change set; the reason is required. */
export async function rejectChangeset(
  workspaceId: string,
  projectId: string,
  changesetId: string,
  reason: string,
): Promise<ChangesetBody> {
  const rejected = await callApi("POST", `${base(workspaceId, projectId, changesetId)}/reject`, {
    body: { reason },
    response: reject.responses["200"],
  });
  return rejected.changeset;
}

const tokens = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/tokens"];

/** The design token tree of the design system's saved version `versionId`, and its modes. */
export async function loadVersionTokens(
  workspaceId: string,
  projectId: string,
  versionId: string,
): Promise<{ modes: string[]; tokens: Record<string, unknown> }> {
  const loaded = await callApi(
    "GET",
    `/api/v1/w/${workspaceId}/projects/${projectId}/tokens?version=${encodeURIComponent(versionId)}`,
    { response: tokens.responses["200"] },
  );
  return { modes: loaded.modes, tokens: loaded.tokens ?? {} };
}
