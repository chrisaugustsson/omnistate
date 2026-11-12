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

/** The ids of the nodes with a question waiting for an answer; a question a person asked is a remark and does not wait. */
export async function loadWaitingNodes(workspaceId: string, projectId: string, featureId: string): Promise<string[]> {
  const loaded = await callApi(
    "GET",
    `/api/v1/w/${workspaceId}/projects/${projectId}/features/${featureId}/behaviour/questions?state=open`,
    { response: openList.responses["200"] },
  );
  return loaded.questions.filter((question) => question.asked_by.kind !== "human").map((question) => question.node_id);
}
