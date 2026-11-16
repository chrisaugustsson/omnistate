import { callApi } from "./client";
import { operations, type DecisionBody } from "./generated";

const ofTrack = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}/decisions"];
const ofFeature =
  operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}/features/{feature_id}/decisions"];
const recorded = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}/decisions"];

const base = (workspaceId: string, projectId: string) => `/api/v1/w/${workspaceId}/projects/${projectId}`;

/** The decisions recorded on a track, newest first. */
export async function loadTrackDecisions(workspaceId: string, projectId: string, trackId: string): Promise<DecisionBody[]> {
  const answer = await callApi("GET", `${base(workspaceId, projectId)}/tracks/${trackId}/decisions`, {
    response: ofTrack.responses["200"],
  });
  return answer.decisions;
}

/** The decisions linked to a feature, newest first. */
export async function loadFeatureDecisions(
  workspaceId: string,
  projectId: string,
  partId: string,
  featureId: string,
): Promise<DecisionBody[]> {
  const answer = await callApi("GET", `${base(workspaceId, projectId)}/parts/${partId}/features/${featureId}/decisions`, {
    response: ofFeature.responses["200"],
  });
  return answer.decisions;
}

/** Records a decision on a track; only the track's owner may. */
export async function recordDecision(
  workspaceId: string,
  projectId: string,
  trackId: string,
  decision: { question: string; decision: string; why: string; feature_id: string | null },
): Promise<DecisionBody> {
  const answer = await callApi("POST", `${base(workspaceId, projectId)}/tracks/${trackId}/decisions`, {
    body: decision,
    response: recorded.responses["201"],
  });
  return answer.decision;
}
