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

/** The name of the project `projectId`, once the workspace's projects are loaded; loaded again when the project is renamed. */
export function useProjectName(workspaceId: string, projectId: string): string | undefined {
  const load = useCallback(() => loadProjects(workspaceId), [workspaceId]);
  const { value, reload } = useReloading(load);
  useProjectEvents(projectId, (event) => {
    if (event.event === "project.renamed") reload();
  });
  return value?.find((project) => project.id === projectId)?.name;
}
