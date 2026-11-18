import { useCallback, useMemo } from "react";
import type { HomeResponse, PartBody, ProjectBody, TrackSummary } from "@/api/generated";
import { loadOpenQuestions } from "@/api/behaviour";
import { inFlight } from "@/components/shell/state-icon";
import { loadHome, loadProjects } from "@/api/home";
import { loadMyUserId } from "@/api/members";
import { loadParts } from "@/api/parts";
import { useProjectsEvents } from "@/live/context";
import { useReloading } from "./reloading";

// What the shell knows across the workspace: its projects, every project's
// home or parts, and who the reader is. Home (S4) aggregates over the same
// hooks, one home per project (build plan, "Data" 6).

/** The workspace's projects; loaded again when `projectId` (the open one) is renamed, and on `reload`. */
export function useProjects(workspaceId: string, projectId?: string) {
  const load = useCallback(() => loadProjects(workspaceId), [workspaceId]);
  const { value: projects, failed, reload } = useReloading(load);
  useProjectsEvents(projectId ? [projectId] : [], (event) => {
    if (event.event === "project.renamed") reload();
  });
  return { projects, failed, reload };
}

/** The signed-in member's user ID in the workspace; null while it loads (and for an agent). */
export function useMe(workspaceId: string): string | null {
  const load = useCallback(() => loadMyUserId(workspaceId), [workspaceId]);
  return useReloading(load).value ?? null;
}

/** Each project's value of `load`, by project ID, loaded again on any live event of any of them. */
function usePerProject<T>(
  workspaceId: string,
  projects: readonly ProjectBody[] | null,
  load: (workspaceId: string, projectId: string) => Promise<T>,
) {
  const ids = useMemo(() => (projects ?? []).map((project) => project.id), [projects]);
  const key = ids.join(",");
  const loadAll = useCallback(async () => {
    const ids = key ? key.split(",") : [];
    const values = await Promise.all(ids.map((projectId) => load(workspaceId, projectId)));
    return new Map(ids.map((projectId, at) => [projectId, values[at]]));
  }, [workspaceId, key, load]);
  const loaded = useReloading(loadAll);
  useProjectsEvents(ids, loaded.reload);
  return { value: projects ? loaded.value : null, failed: loaded.failed, reload: loaded.reload };
}

/** Every project's home (tracks, docs, threads, what waits), by project ID; null while they load. */
export function useWorkspaceHomes(workspaceId: string, projects: readonly ProjectBody[] | null) {
  const { value: homes, failed, reload } = usePerProject<HomeResponse>(workspaceId, projects, loadHome);
  return { homes, failed, reload };
}

/** Every project's parts, by project ID; null while they load. */
export function useWorkspaceParts(workspaceId: string, projects: readonly ProjectBody[] | null) {
  const { value: parts, failed, reload } = usePerProject<PartBody[]>(workspaceId, projects, loadParts);
  return { parts, failed, reload };
}

/**
 * How many comments wait on the reader across the workspace: those whose latest
 * reply is an agent's, on tracks the reader owns (the Inbox, decision U8).
 * Null while it is not known.
 */
export function waitingOnMe(homes: ReadonlyMap<string, HomeResponse> | null, me: string | null): number | null {
  if (!homes || !me) return null;
  let count = 0;
  for (const home of homes.values()) {
    const mine = new Set((home.tracks ?? []).filter(({ track }) => track.owner_user_id === me).map(({ track }) => track.id));
    count += home.waiting.comments.filter((comment) => comment.track_id && mine.has(comment.track_id)).length;
  }
  return count;
}

/**
 * The open behaviour questions an agent asked on each of `tracks` in flight,
 * by track ID (a person's remark waits on no one); read again on any
 * behaviour event of their projects.
 */
export function useOpenQuestions(workspaceId: string, tracks: readonly TrackSummary[] | null) {
  const live = useMemo(() => (tracks ?? []).filter(({ track }) => inFlight(track.state)), [tracks]);
  const key = live.map(({ track }) => `${track.project_id}/${track.id}`).join(",");
  const load = useCallback(async () => {
    const pairs = key ? key.split(",").map((pair) => pair.split("/") as [string, string]) : [];
    const lists = await Promise.all(
      pairs.map(async ([projectId, trackId]) => {
        const questions = await loadOpenQuestions(workspaceId, projectId, trackId).catch(() => []);
        return [trackId, questions.filter((question) => question.asked_by.kind !== "human")] as const;
      }),
    );
    return new Map(lists);
  }, [workspaceId, key]);
  const { value, reload } = useReloading(load);
  const projectIds = useMemo(() => [...new Set(live.map(({ track }) => track.project_id))], [live]);
  useProjectsEvents(projectIds, (event) => {
    if (event.event.startsWith("behaviour.")) reload();
  });
  return tracks ? (value ?? null) : null;
}
