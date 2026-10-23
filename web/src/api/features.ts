import { callApi } from "./client";
import { operations, type FeatureBody } from "./generated";

const base = (workspaceId: string, projectId: string, featureId: string) =>
  `/api/v1/w/${workspaceId}/projects/${projectId}/features/${featureId}`;

const done = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/done"];
const reopen = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/reopen"];

/** Marks the feature done: it takes no new change sets, and its docs stay as the record. */
export async function markFeatureDone(workspaceId: string, projectId: string, featureId: string): Promise<FeatureBody> {
  const marked = await callApi("POST", `${base(workspaceId, projectId, featureId)}/done`, {
    body: {},
    response: done.responses["200"],
  });
  return marked.feature;
}

/** Makes a done feature active again. */
export async function reopenFeature(workspaceId: string, projectId: string, featureId: string): Promise<FeatureBody> {
  const reopened = await callApi("POST", `${base(workspaceId, projectId, featureId)}/reopen`, {
    body: {},
    response: reopen.responses["200"],
  });
  return reopened.feature;
}
