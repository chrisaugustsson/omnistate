import { useCallback } from "react";
import { loadHome, loadProjects } from "@/api/home";
import { useProjectEvents } from "@/live/context";
import { useReloading } from "./reloading";

/**
 * A project's home, loaded again on every live event of the project, so what
 * it lists appears and disappears without a reload.
 */
export function useHome(workspaceId: string, projectId: string) {
  const load = useCallback(() => loadHome(workspaceId, projectId), [workspaceId, projectId]);
  const { value: home, failed, reload } = useReloading(load);
  useProjectEvents(projectId, reload);
  return { home, failed, reload };
}

/** The name of the project `projectId`, once the workspace's projects are loaded. */
export function useProjectName(workspaceId: string, projectId: string): string | undefined {
  const load = useCallback(() => loadProjects(workspaceId), [workspaceId]);
  const { value } = useReloading(load);
  return value?.find((project) => project.id === projectId)?.name;
}
