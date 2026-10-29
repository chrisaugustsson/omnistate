import { callApi } from "./client";
import { operations, type ApiKeyBody, type CreatedKeyResponse } from "./generated";

const list = operations["GET /api/v1/w/{workspace_id}/api-keys"];
const create = operations["POST /api/v1/w/{workspace_id}/api-keys"];
const revoke = operations["POST /api/v1/w/{workspace_id}/api-keys/{key_id}/revoke"];

/** The workspace's API keys, with revoked ones. */
export async function loadKeys(workspaceId: string): Promise<ApiKeyBody[]> {
  const loaded = await callApi("GET", `/api/v1/w/${workspaceId}/api-keys`, { response: list.responses["200"] });
  return loaded.keys;
}

/** Creates a key; the answer holds its secret, which is never shown again. */
export function createKey(workspaceId: string): Promise<CreatedKeyResponse> {
  return callApi("POST", `/api/v1/w/${workspaceId}/api-keys`, { response: create.responses["201"] });
}

/** Revokes a key: the agents that use it are refused from then on. */
export async function revokeKey(workspaceId: string, keyId: string): Promise<void> {
  await callApi("POST", `/api/v1/w/${workspaceId}/api-keys/${keyId}/revoke`, { response: revoke.responses["200"] });
}
