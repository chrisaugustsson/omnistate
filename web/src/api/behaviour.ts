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

const read = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/nodes/{node_id}"];

/** One node in full: its text, questions with all their answers, and the rules that reach it. */
export function loadNode(workspaceId: string, projectId: string, featureId: string, nodeId: string) {
  return callApi(
    "GET",
    `/api/v1/w/${workspaceId}/projects/${projectId}/features/${featureId}/behaviour/nodes/${encodeURIComponent(nodeId)}`,
    { response: read.responses["200"] },
  );
}

const answer = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/questions/{question_id}/answers"];

/** A person's answer to a question; `change` replaces an answer already given. A 409 means someone got there first. */
export async function answerQuestion(
  workspaceId: string,
  projectId: string,
  featureId: string,
  questionId: string,
  body: { expected_version: number; answer: string; change?: boolean },
) {
  const done = await callApi(
    "POST",
    `/api/v1/w/${workspaceId}/projects/${projectId}/features/${featureId}/behaviour/questions/${encodeURIComponent(questionId)}/answers`,
    { body, response: answer.responses["201"] },
  );
  return done.question;
}

const ask = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/behaviour/nodes/{node_id}/questions"];

/** A person's question or remark on a node; it opens the node again for the next grill pass. */
export async function askQuestion(
  workspaceId: string,
  projectId: string,
  featureId: string,
  nodeId: string,
  body: { question: string },
) {
  const done = await callApi(
    "POST",
    `/api/v1/w/${workspaceId}/projects/${projectId}/features/${featureId}/behaviour/nodes/${encodeURIComponent(nodeId)}/questions`,
    { body, response: ask.responses["201"] },
  );
  return done.question;
}
