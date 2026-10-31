import { callApi } from "./client";
import { operations, type CreateStoryRequest, type PatchStoryRequest, type StoryBody } from "./generated";

const list = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories"];

/** Every story of a feature, any status, by priority then number. */
export async function loadStories(workspaceId: string, projectId: string, featureId: string): Promise<StoryBody[]> {
  const loaded = await callApi(
    "GET",
    `/api/v1/w/${workspaceId}/projects/${projectId}/features/${featureId}/stories`,
    { response: list.responses["200"] },
  );
  return loaded.stories;
}

const base = (workspaceId: string, projectId: string, featureId: string) =>
  `/api/v1/w/${workspaceId}/projects/${projectId}/features/${featureId}/stories`;
const prefix = "/api/v1/w/{workspace_id}/projects/{project_id}/features/{feature_id}/stories";
const created = operations[`POST ${prefix}`];
const patched = operations[`PATCH ${prefix}/{story_id}`];
const cancelled = operations[`POST ${prefix}/{story_id}/cancel`];
const checked = operations[`POST ${prefix}/{story_id}/check`];

/** Adds a story to the backlog of a feature. */
export async function createStory(
  workspaceId: string,
  projectId: string,
  featureId: string,
  body: CreateStoryRequest,
): Promise<StoryBody> {
  const answer = await callApi("POST", base(workspaceId, projectId, featureId), { body, response: created.responses["201"] });
  return answer.story;
}

/** Changes fields of a story; `expected_version` must be the card's version. */
export async function patchStory(
  workspaceId: string,
  projectId: string,
  featureId: string,
  storyId: string,
  body: PatchStoryRequest,
): Promise<StoryBody> {
  const answer = await callApi("PATCH", `${base(workspaceId, projectId, featureId)}/${storyId}`, {
    body,
    response: patched.responses["200"],
  });
  return answer.story;
}

/** Cancels a story. */
export async function cancelStory(
  workspaceId: string,
  projectId: string,
  featureId: string,
  storyId: string,
  expectedVersion: number,
): Promise<StoryBody> {
  const answer = await callApi("POST", `${base(workspaceId, projectId, featureId)}/${storyId}/cancel`, {
    body: { expected_version: expectedVersion },
    response: cancelled.responses["200"],
  });
  return answer.story;
}

/** Clears the pending human check of one criterion (0-based). */
export async function checkCriterion(
  workspaceId: string,
  projectId: string,
  featureId: string,
  storyId: string,
  expectedVersion: number,
  criterion: number,
): Promise<StoryBody> {
  const answer = await callApi("POST", `${base(workspaceId, projectId, featureId)}/${storyId}/check`, {
    body: { expected_version: expectedVersion, criterion },
    response: checked.responses["200"],
  });
  return answer.story;
}
