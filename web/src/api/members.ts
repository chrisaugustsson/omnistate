import { callApi } from "./client";
import { operations, type MemberBody, type TrackBody } from "./generated";

const members = operations["GET /api/v1/w/{workspace_id}/members"];
const whoami = operations["GET /api/v1/w/{workspace_id}/whoami"];
const handedOver = operations["POST /api/v1/w/{workspace_id}/projects/{project_id}/tracks/{track_id}/owner"];

/** The members of the workspace, in the order they joined. */
export async function loadMembers(workspaceId: string): Promise<MemberBody[]> {
  const answer = await callApi("GET", `/api/v1/w/${workspaceId}/members`, { response: members.responses["200"] });
  return answer.members;
}

/** The logged-in user's ID in the workspace. */
export async function loadMyUserId(workspaceId: string): Promise<string | null> {
  const answer = await callApi("GET", `/api/v1/w/${workspaceId}/whoami`, { response: whoami.responses["200"] });
  return answer.actor.user_id ?? null;
}

/** Hands a track to another member; only the owner or an admin may. */
export async function handOverTrack(
  workspaceId: string,
  projectId: string,
  trackId: string,
  membershipId: string,
): Promise<TrackBody> {
  const answer = await callApi("POST", `/api/v1/w/${workspaceId}/projects/${projectId}/tracks/${trackId}/owner`, {
    body: { membership_id: membershipId },
    response: handedOver.responses["200"],
  });
  return answer.track;
}
