import { useCallback } from "react";
import { loadFeatures } from "@/api/features";
import { useProjectEvents } from "@/live/context";
import { useReloading } from "./reloading";

/** A part's undeleted features, loaded again on every live event of the project. */
export function useFeatures(workspaceId: string, projectId: string, partId: string) {
  const load = useCallback(() => loadFeatures(workspaceId, projectId, partId), [workspaceId, projectId, partId]);
  const { value: features, failed, reload } = useReloading(load, `features:${partId}`);
  useProjectEvents(projectId, reload);
  return { features, failed, reload };
}
