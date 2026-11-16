import { callApi } from "./client";
import { operations, type DocBody, type FeatureBody, type TrackBody } from "./generated";

const list = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features"];
const created = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features"];
const one = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features/{feature_id}"];
const renamed = operations["PATCH /api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features/{feature_id}"];
const linkedTracks =
  operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features/{feature_id}/tracks"];
const ofTrack = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}/features"];
const docs = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/docs"];

const base = (workspaceId: string, projectId: string) => `/api/v1/w/${workspaceId}/projects/${projectId}`;
const at = (workspaceId: string, projectId: string, partId: string) =>
  `${base(workspaceId, projectId)}/parts/${partId}/features`;

/** The undeleted features of a part. */
export async function loadFeatures(workspaceId: string, projectId: string, partId: string): Promise<FeatureBody[]> {
  const answer = await callApi("GET", at(workspaceId, projectId, partId), { response: list.responses["200"] });
  return answer.features;
}

export async function loadFeature(
  workspaceId: string,
  projectId: string,
  partId: string,
  featureId: string,
): Promise<FeatureBody> {
  const answer = await callApi("GET", `${at(workspaceId, projectId, partId)}/${featureId}`, {
    response: one.responses["200"],
  });
  return answer.feature;
}

/** Creates a feature; the server trims the name. */
export async function createFeature(
  workspaceId: string,
  projectId: string,
  partId: string,
  feature: { name: string; description: string },
): Promise<FeatureBody> {
  const answer = await callApi("POST", at(workspaceId, projectId, partId), {
    body: feature,
    response: created.responses["201"],
  });
  return answer.feature;
}

export async function renameFeature(
  workspaceId: string,
  projectId: string,
  partId: string,
  featureId: string,
  name: string,
): Promise<FeatureBody> {
  const answer = await callApi("PATCH", `${at(workspaceId, projectId, partId)}/${featureId}`, {
    body: { name },
    response: renamed.responses["200"],
  });
  return answer.feature;
}

/** Deletes (hides) a feature. */
export function deleteFeature(workspaceId: string, projectId: string, partId: string, featureId: string): Promise<void> {
  return callApi("DELETE", `${at(workspaceId, projectId, partId)}/${featureId}`);
}

/** The tracks linked to a feature. */
export async function loadFeatureTracks(
  workspaceId: string,
  projectId: string,
  partId: string,
  featureId: string,
): Promise<TrackBody[]> {
  const answer = await callApi("GET", `${at(workspaceId, projectId, partId)}/${featureId}/tracks`, {
    response: linkedTracks.responses["200"],
  });
  return answer.tracks;
}

/** The docs that sit on a feature. */
export async function loadFeatureDocs(workspaceId: string, projectId: string, featureId: string): Promise<DocBody[]> {
  const answer = await callApi("GET", `${base(workspaceId, projectId)}/docs?feature=${featureId}`, {
    response: docs.responses["200"],
  });
  return answer.docs;
}

/** The docs that sit on a part itself, not those of its features or tracks. */
export async function loadPartDocs(workspaceId: string, projectId: string, partId: string): Promise<DocBody[]> {
  const answer = await callApi("GET", `${base(workspaceId, projectId)}/docs?part=${partId}`, {
    response: docs.responses["200"],
  });
  return answer.docs;
}

/** The features that a track is linked to. */
export async function loadTrackFeatures(workspaceId: string, projectId: string, trackId: string): Promise<FeatureBody[]> {
  const answer = await callApi("GET", `${base(workspaceId, projectId)}/tracks/${trackId}/features`, {
    response: ofTrack.responses["200"],
  });
  return answer.features;
}
