import { callApi } from "./client";
import { operations, type CreateStoryRequest, type LinkedBody, type PatchStoryRequest, type StoryBody } from "./generated";

const list = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}/stories"];

/** Every story of a track, any status, by priority then number, and the stories of other tracks linked to them. */
export async function loadStories(
  workspaceId: string,
  projectId: string,
  trackId: string,
): Promise<{ stories: StoryBody[]; linked: LinkedBody[] }> {
  const loaded = await callApi(
    "GET",
    `/api/v1/w/${workspaceId}/projects/${projectId}/tracks/${trackId}/stories`,
    { response: list.responses["200"] },
  );
  return { stories: loaded.stories, linked: loaded.linked ?? [] };
}

const base = (workspaceId: string, projectId: string, trackId: string) =>
  `/api/v1/w/${workspaceId}/projects/${projectId}/tracks/${trackId}/stories`;
const prefix = "/api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}/stories";
const created = operations[`POST ${prefix}`];
const patched = operations[`PATCH ${prefix}/{story_id}`];
const cancelled = operations[`POST ${prefix}/{story_id}/cancel`];
const checked = operations[`POST ${prefix}/{story_id}/check`];

/** Adds a story to the backlog of a track. */
export async function createStory(
  workspaceId: string,
  projectId: string,
  trackId: string,
  body: CreateStoryRequest,
): Promise<StoryBody> {
  const answer = await callApi("POST", base(workspaceId, projectId, trackId), { body, response: created.responses["201"] });
  return answer.story;
}

/** Changes fields of a story; `expected_version` must be the card's version. */
export async function patchStory(
  workspaceId: string,
  projectId: string,
  trackId: string,
  storyId: string,
  body: PatchStoryRequest,
): Promise<StoryBody> {
  const answer = await callApi("PATCH", `${base(workspaceId, projectId, trackId)}/${storyId}`, {
    body,
    response: patched.responses["200"],
  });
  return answer.story;
}

/** Cancels a story. */
export async function cancelStory(
  workspaceId: string,
  projectId: string,
  trackId: string,
  storyId: string,
  expectedVersion: number,
): Promise<StoryBody> {
  const answer = await callApi("POST", `${base(workspaceId, projectId, trackId)}/${storyId}/cancel`, {
    body: { expected_version: expectedVersion },
    response: cancelled.responses["200"],
  });
  return answer.story;
}

/** Clears the pending human check of one criterion (0-based). */
export async function checkCriterion(
  workspaceId: string,
  projectId: string,
  trackId: string,
  storyId: string,
  expectedVersion: number,
  criterion: number,
): Promise<StoryBody> {
  const answer = await callApi("POST", `${base(workspaceId, projectId, trackId)}/${storyId}/check`, {
    body: { expected_version: expectedVersion, criterion },
    response: checked.responses["200"],
  });
  return answer.story;
}
