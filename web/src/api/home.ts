import { callApi } from "./client";
import { operations, type HomeResponse, type ProjectBody } from "./generated";

const projects = operations["GET /api/v1/w/{workspace_id}/projects"];
const home = operations["GET /api/v1/w/{workspace_id}/projects/{project_id}/home"];

/** The projects of a workspace. */
export async function loadProjects(workspaceId: string): Promise<ProjectBody[]> {
  const list = await callApi("GET", `/api/v1/w/${workspaceId}/projects`, {
    response: projects.responses["200"],
  });
  return list.projects;
}

/** The project's home: its docs, change sets and threads, and what waits for the human. */
export function loadHome(workspaceId: string, projectId: string): Promise<HomeResponse> {
  return callApi("GET", `/api/v1/w/${workspaceId}/projects/${projectId}/home`, {
    response: home.responses["200"],
  });
}
