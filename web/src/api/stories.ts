import { callApi } from "./client";
import { operations, type StoryBody } from "./generated";

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
