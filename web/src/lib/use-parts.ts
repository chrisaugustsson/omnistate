import { useCallback } from "react";
import { loadParts } from "@/api/parts";
import { useProjectEvents } from "@/live/context";
import { useReloading } from "./reloading";

/** A project's undeleted parts, loaded again on every live event of the project. */
export function useParts(workspaceId: string, projectId: string) {
  const load = useCallback(() => loadParts(workspaceId, projectId), [workspaceId, projectId]);
  const { value: parts, failed, reload } = useReloading(load);
  useProjectEvents(projectId, reload);
  return { parts, failed, reload };
}
