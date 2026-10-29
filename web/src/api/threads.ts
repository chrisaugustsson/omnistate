import { callApi } from "./client";
import { operations, type PostBody, type ThreadBody } from "./generated";

const base = (workspaceId: string, projectId: string) => `/api/v1/w/${workspaceId}/projects/${projectId}`;

const thread = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}"];
const post = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/threads/{thread_id}/posts"];

/** A thread as the human reads it: every post, oldest first. */
export async function loadThread(workspaceId: string, projectId: string, threadId: string): Promise<ThreadBody> {
  const loaded = await callApi("GET", `${base(workspaceId, projectId)}/threads/${threadId}`, {
    response: thread.responses["200"],
  });
  return loaded.thread;
}

/** Posts in a thread; the summary and the Markdown body are sent as they are. */
export async function createPost(
  workspaceId: string,
  projectId: string,
  threadId: string,
  fields: { summary: string; body: string },
): Promise<PostBody> {
  const created = await callApi("POST", `${base(workspaceId, projectId)}/threads/${threadId}/posts`, {
    body: fields,
    response: post.responses["201"],
  });
  return created.post;
}
