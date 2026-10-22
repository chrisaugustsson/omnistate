import { useCallback } from "react";
import { loadHome } from "@/api/home";
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
  return { home, failed };
}
