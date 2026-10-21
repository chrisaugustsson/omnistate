import { ApiError, callApi } from "./client";
import { operations, type LoginRequest, type UserBody, type WorkspaceBody } from "./generated";

const login = operations["POST /api/v1/auth/login"];
const me = operations["GET /api/v1/auth/me"];
const workspaces = operations["GET /api/v1/auth/workspaces"];

export type Session = { user: UserBody; workspaces: WorkspaceBody[] };

/** Logs in; the API sets the session cookie. A refusal is an ApiError (401 invalid_credentials). */
export async function logIn(credentials: LoginRequest): Promise<UserBody> {
  const { user } = await callApi("POST", "/api/v1/auth/login", {
    body: login.request.parse(credentials),
    response: login.responses["200"],
  });
  return user;
}

/** Ends the session through the API. */
export async function logOut(): Promise<void> {
  await callApi("POST", "/api/v1/auth/logout");
}

/** The logged-in user and their workspaces, or null without a session. */
export async function loadSession(): Promise<Session | null> {
  try {
    const { user } = await callApi("GET", "/api/v1/auth/me", { response: me.responses["200"] });
    const list = await callApi("GET", "/api/v1/auth/workspaces", {
      response: workspaces.responses["200"],
    });
    return { user, workspaces: list.workspaces };
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) return null;
    throw error;
  }
}
