import { callApi } from "./client";
import { operations, type PartBody, type TrackBody } from "./generated";

const list = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/parts"];
const created = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/parts"];
const renamed = operations["PATCH /api/v1/w/{workspace_id}/projects/{project_id}/parts/{part_id}"];
const createdTrack = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/tracks"];
const movedTrack = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}/state"];

const base = (workspaceId: string, projectId: string) => `/api/v1/w/${workspaceId}/projects/${projectId}`;

/** The undeleted parts of a project, in the order they were created. */
export async function loadParts(workspaceId: string, projectId: string): Promise<PartBody[]> {
  const answer = await callApi("GET", `${base(workspaceId, projectId)}/parts`, { response: list.responses["200"] });
  return answer.parts;
}

/** Creates a part; the server trims the name. */
export async function createPart(
  workspaceId: string,
  projectId: string,
  part: { name: string; kind: string; description: string },
): Promise<PartBody> {
  const answer = await callApi("POST", `${base(workspaceId, projectId)}/parts`, {
    body: part,
    response: created.responses["201"],
  });
  return answer.part;
}

export async function renamePart(workspaceId: string, projectId: string, partId: string, name: string): Promise<PartBody> {
  const answer = await callApi("PATCH", `${base(workspaceId, projectId)}/parts/${partId}`, {
    body: { name },
    response: renamed.responses["200"],
  });
  return answer.part;
}

/** Sets a part's description, the intro its Overview shows; an empty one clears it. */
export async function describePart(workspaceId: string, projectId: string, partId: string, description: string): Promise<PartBody> {
  const answer = await callApi("PATCH", `${base(workspaceId, projectId)}/parts/${partId}`, {
    body: { description },
    response: renamed.responses["200"],
  });
  return answer.part;
}

/** Deletes (hides) a part; its tracks stay. */
export function deletePart(workspaceId: string, projectId: string, partId: string): Promise<void> {
  return callApi("DELETE", `${base(workspaceId, projectId)}/parts/${partId}`);
}

/** Creates a track on a part of the project. */
export async function createTrack(
  workspaceId: string,
  projectId: string,
  track: { name: string; description: string; part_id: string },
): Promise<TrackBody> {
  const answer = await callApi("POST", `${base(workspaceId, projectId)}/tracks`, {
    body: track,
    response: createdTrack.responses["201"],
  });
  return answer.track;
}

/** Moves a track to another state; the API refuses a move that is not allowed (US-311). */
export async function setTrackState(workspaceId: string, projectId: string, trackId: string, state: string): Promise<TrackBody> {
  const answer = await callApi("POST", `${base(workspaceId, projectId)}/tracks/${trackId}/state`, {
    body: { state },
    response: movedTrack.responses["200"],
  });
  return answer.track;
}
