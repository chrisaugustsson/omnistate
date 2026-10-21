import { callApi } from "./client";
import { operations, type DecisionResponse, type PostBody, type ThreadBody } from "./generated";

const base = (workspaceId: string, projectId: string) => `/api/v1/w/${workspaceId}/projects/${projectId}`;

const thread = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}"];
const post = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/posts"];
const directive = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/directives"];
const approve = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}/approve"];
const reject = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/decisions/{decision_id}/reject"];

/** A link from a post to what it answers. */
export type PostLink = { kind: "post"; post_id: string };

/** A thread as the human reads it: every post of every phase, and its current decision. */
export async function loadThread(workspaceId: string, projectId: string, threadId: string): Promise<ThreadBody> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/threads/${threadId}`, {
    response: thread.responses["200"],
  });
  return loaded.thread;
}

/** Posts a note or question; `links` makes it a reply. */
export async function createPost(
  workspaceId: string,
  projectId: string,
  threadId: string,
  fields: { type: "note" | "question"; summary: string; body: string; links: PostLink[] },
): Promise<PostBody> {
  const created = await callApi("POST", `${base(workspaceId, projectId)}/threads/${threadId}/posts`, {
    body: fields,
    response: post.responses["201"],
  });
  return created.post;
}

/** Posts a directive, which only the human may. */
export async function createDirective(
  workspaceId: string,
  projectId: string,
  threadId: string,
  fields: { summary: string; body: string; links: PostLink[] },
): Promise<PostBody> {
  const created = await callApi("POST", `${base(workspaceId, projectId)}/threads/${threadId}/directives`, {
    body: fields,
    response: directive.responses["201"],
  });
  return created.post;
}

/** Approves a proposed decision. */
export function approveDecision(workspaceId: string, projectId: string, decisionId: string): Promise<DecisionResponse> {
  return callApi("POST", `${base(workspaceId, projectId)}/decisions/${decisionId}/approve`, {
    response: approve.responses["200"],
  });
}

/** Rejects a proposed decision; the reason is required. */
export function rejectDecision(
  workspaceId: string,
  projectId: string,
  decisionId: string,
  reason: string,
): Promise<DecisionResponse> {
  return callApi("POST", `${base(workspaceId, projectId)}/decisions/${decisionId}/reject`, {
    body: { reason },
    response: reject.responses["200"],
  });
}
