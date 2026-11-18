import { useCallback, useMemo } from "react";
import { useRouteContext } from "@tanstack/react-router";
import type { PartBody, TrackBody } from "@/api/generated";
import { loadOpenQuestions } from "@/api/behaviour";
import { loadMembers } from "@/api/members";
import { loadStories } from "@/api/stories";
import { useProjectsEvents } from "@/live/context";
import { useReloading } from "@/lib/reloading";
import { useMe, useProjects, useWorkspaceHomes, useWorkspaceParts } from "@/lib/use-workspace";
import { inFlight } from "@/components/shell/state-icon";

// What Home's three pages load: every project of the workspace with its home
// and parts (one call each), the members (for names), and who the reader is.

export function useHomeData(workspaceId: string) {
  const { user, workspace } = useRouteContext({ from: "/w/$workspaceId" });
  const { projects, failed: projectsFailed } = useProjects(workspaceId);
  const { homes, failed: homesFailed, reload } = useWorkspaceHomes(workspaceId, projects);
  const { parts } = useWorkspaceParts(workspaceId, projects);
  const me = useMe(workspaceId);
  const { value: members } = useReloading(useCallback(() => loadMembers(workspaceId), [workspaceId]), `members:${workspaceId}`);
  return { user, workspace, projects, homes, parts, me, members, reload, failed: projectsFailed || homesFailed };
}

/** A part by its project and ID, from the workspace's parts. */
export function findPart(parts: ReadonlyMap<string, PartBody[]> | null, projectId: string, partId: string): PartBody | undefined {
  return parts?.get(projectId)?.find((part) => part.id === partId);
}

/** What a track's row flags: its open behaviour questions, and the first of its stories that waits on another track's. */
export type TrackFlags = { questions: number; blocked: string | null };

/**
 * The flags of the tracks in flight among `tracks`: how many behaviour
 * questions an agent asked that wait for an answer (a person's remark does not
 * wait), and whether a story not yet done is blocked, as "US-1 blocked by
 * iot-service US-2". Read again on any live event of their projects.
 */
export function useTrackFlags(workspaceId: string, tracks: readonly TrackBody[] | null): ReadonlyMap<string, TrackFlags> {
  const live = useMemo(() => (tracks ?? []).filter((track) => inFlight(track.state)), [tracks]);
  const key = live.map((track) => `${track.project_id}/${track.id}`).join(",");
  const load = useCallback(async () => {
    const pairs = key ? key.split(",").map((pair) => pair.split("/") as [string, string]) : [];
    const flags = await Promise.all(
      pairs.map(async ([projectId, trackId]): Promise<[string, TrackFlags]> => {
        const [questions, stories] = await Promise.all([
          loadOpenQuestions(workspaceId, projectId, trackId).catch(() => []),
          loadStories(workspaceId, projectId, trackId).catch(() => ({ stories: [] })),
        ]);
        const open = (status: string) => status !== "done" && status !== "cancelled";
        const waiting = stories.stories
          .filter((story) => open(story.status))
          .map((story) => ({ story, blocker: story.blockers?.find((blocker) => open(blocker.status)) }));
        const first = waiting.find(({ blocker }) => blocker) ?? waiting.find(({ story }) => story.blocked);
        const blocked = first
          ? first.blocker
            ? `${first.story.key} blocked by ${first.blocker.track_name} ${first.blocker.key}`
            : `${first.story.key} is blocked`
          : null;
        return [trackId, { questions: questions.filter((question) => question.asked_by.kind !== "human").length, blocked }];
      }),
    );
    return new Map(flags);
  }, [workspaceId, key]);
  const { value, reload } = useReloading(load, `row-flags:${workspaceId}:${key}`);
  const projectIds = useMemo(() => [...new Set(live.map((track) => track.project_id))], [live]);
  useProjectsEvents(projectIds, (event) => {
    if (event.event.startsWith("behaviour.") || event.event.startsWith("story.")) reload();
  });
  return value ?? EMPTY;
}

const EMPTY: ReadonlyMap<string, TrackFlags> = new Map();
