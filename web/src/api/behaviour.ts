import { callApi } from "./client";
import { operations, type NodeSummaryBody } from "./generated";

export type TreeNode = NodeSummaryBody;

const list = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour"];

/** Every node of a feature's behaviour tree, the root first, in tree order. */
export async function loadTree(workspaceId: string, projectId: string, featureId: string): Promise<TreeNode[]> {
  const loaded = await callApi(
    "GET",
    `/api/v1/w/${workspaceId}/projects/${projectId}/features/${featureId}/behaviour`,
    { response: list.responses["200"] },
  );
  return loaded.nodes;
}

const openList = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/questions"];

/** The open questions of a feature's tree, a remark a person left among them. */
export async function loadOpenQuestions(workspaceId: string, projectId: string, featureId: string) {
  const loaded = await callApi(
    "GET",
    `/api/v1/w/${workspaceId}/projects/${projectId}/features/${featureId}/behaviour/questions?state=open`,
    { response: openList.responses["200"] },
  );
  return loaded.questions;
}
