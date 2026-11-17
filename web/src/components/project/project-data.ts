import { useCallback, useMemo } from "react";
import { loadOpenQuestions } from "@/api/behaviour";
import { loadFeatureDecisions, loadPartDecisions, loadProjectDecisions } from "@/api/decisions";
import { loadFeatureTracks, loadFeatures } from "@/api/features";
import type { DecisionBody, FeatureBody, PartBody, TrackSummary } from "@/api/generated";
import { loadMembers } from "@/api/members";
import { loadParts } from "@/api/parts";
import { personName } from "@/lib/identity";
import { useReloading } from "@/lib/reloading";
import { useProjectEvents } from "@/live/context";

// What the project's and the part's pages read beyond the project's home: the
// parts with their features and which tracks change each feature, the
// decisions of every level, the members' names and the open questions of the
// tracks in flight. Each is loaded again on the project's live events.

/** A project's parts, each part's features, and the features each track changes. */
export type ProjectMap = {
  parts: PartBody[];
  /** The features of each part, by part ID, in the order they were created. */
  features: ReadonlyMap<string, FeatureBody[]>;
  /** The features each track changes, by track ID. */
  changes: ReadonlyMap<string, FeatureBody[]>;
  /** The IDs of the tracks that change each feature, by feature ID. */
  changedBy: ReadonlyMap<string, string[]>;
};

async function loadProjectMap(workspaceId: string, projectId: string): Promise<ProjectMap> {
  const parts = await loadParts(workspaceId, projectId);
  const lists = await Promise.all(parts.map((part) => loadFeatures(workspaceId, projectId, part.id)));
  const features = new Map(parts.map((part, at) => [part.id, lists[at]]));
  const all = lists.flat();
  const tracks = await Promise.all(all.map((feature) => loadFeatureTracks(workspaceId, projectId, feature.part_id, feature.id)));
  const changes = new Map<string, FeatureBody[]>();
  const changedBy = new Map<string, string[]>();
  all.forEach((feature, at) => {
    changedBy.set(
      feature.id,
      tracks[at].map((track) => track.id),
    );
    for (const track of tracks[at]) changes.set(track.id, [...(changes.get(track.id) ?? []), feature]);
  });
  return { parts, features, changes, changedBy };
}

/** The project's map (parts, features, what each track changes); null while it loads. */
export function useProjectMap(workspaceId: string, projectId: string) {
  const load = useCallback(() => loadProjectMap(workspaceId, projectId), [workspaceId, projectId]);
  const { value, failed, reload } = useReloading(load);
  useProjectEvents(projectId, reload);
  return { map: value, failed, reload };
}

/** What a group of decisions is about: the project, a part or a feature. */
export type Subject =
  | { kind: "project"; id: string; name: string }
  | { kind: "part"; id: string; name: string; part: PartBody }
  | { kind: "feature"; id: string; name: string; part: PartBody; feature: FeatureBody };

/** The decisions about one subject: those of its level, and those of other levels they replace. */
export type DecisionGroup = { subject: Subject; decisions: DecisionBody[]; earlier: DecisionBody[] };

/**
 * The decisions of a level and the levels under it, one group per subject: for
 * the project its own, then each part's followed by its features'; for a part
 * its own, then its features'. The groups come in that order.
 */
async function loadDecisionGroups(
  workspaceId: string,
  projectId: string,
  projectName: string,
  map: ProjectMap,
  partId: string | undefined,
): Promise<DecisionGroup[]> {
  const subjects: Subject[] = [];
  if (!partId) subjects.push({ kind: "project", id: projectId, name: projectName });
  for (const part of map.parts) {
    if (partId && part.id !== partId) continue;
    subjects.push({ kind: "part", id: part.id, name: part.name, part });
    for (const feature of map.features.get(part.id) ?? []) {
      subjects.push({ kind: "feature", id: feature.id, name: feature.name, part, feature });
    }
  }
  const lists = await Promise.all(
    subjects.map((subject) =>
      subject.kind === "project"
        ? loadProjectDecisions(workspaceId, projectId)
        : subject.kind === "part"
          ? loadPartDecisions(workspaceId, projectId, subject.id)
          : loadFeatureDecisions(workspaceId, projectId, subject.part.id, subject.id),
    ),
  );
  return subjects.map((subject, at) => ({ subject, ...lists[at] }));
}

/** The decision groups of the project (no `partId`) or of a part; null while they or the map load. */
export function useDecisionGroups(
  workspaceId: string,
  projectId: string,
  projectName: string | undefined,
  map: ProjectMap | null,
  partId?: string,
) {
  const load = useCallback(
    async () => (map && projectName !== undefined ? loadDecisionGroups(workspaceId, projectId, projectName, map, partId) : null),
    [workspaceId, projectId, projectName, map, partId],
  );
  const { value, failed, reload } = useReloading(load);
  useProjectEvents(projectId, reload);
  return { groups: value, failed };
}

/** Whether a later decision replaced this one. */
export const isReplaced = (decision: DecisionBody) => Boolean(decision.replaced_by_decision_id);

/** The decisions of the groups nobody replaced, newest first. */
export function currentDecisions(groups: readonly DecisionGroup[]): { decision: DecisionBody; subject: Subject }[] {
  return groups
    .flatMap((group) => group.decisions.filter((decision) => !isReplaced(decision)).map((decision) => ({ decision, subject: group.subject })))
    .sort((a, b) => Date.parse(b.decision.created_at) - Date.parse(a.decision.created_at) || b.decision.id.localeCompare(a.decision.id));
}

/** The workspace's members' names by user ID (decision U8: the email before the @, capitalised). */
export function useMemberNames(workspaceId: string): ReadonlyMap<string, string> | null {
  const load = useCallback(() => loadMembers(workspaceId), [workspaceId]);
  const { value } = useReloading(load);
  return useMemo(() => (value ? new Map(value.map((member) => [member.user_id, personName(member.email)])) : null), [value]);
}

/** Who wrote something, from its `author`: a member's name, or "<role> agent". */
export function authorOf(author: Record<string, unknown>, names: ReadonlyMap<string, string> | null): { name: string; agent: boolean; key: string } {
  if (author.kind === "human") {
    const id = typeof author.user_id === "string" ? author.user_id : "";
    return { name: names?.get(id) ?? "Someone", agent: false, key: id };
  }
  const role = typeof author.role === "string" && author.role ? author.role : "an";
  const name = `${role} agent`;
  return { name: name.charAt(0).toUpperCase() + name.slice(1), agent: true, key: name };
}

/** How many questions wait on a person in each of `tracks`' trees, by track ID; tracks not in flight are not asked. */
export function useOpenQuestions(workspaceId: string, projectId: string, tracks: readonly TrackSummary[] | null) {
  const ids = (tracks ?? [])
    .filter(({ track }) => track.state === "planning" || track.state === "building")
    .map(({ track }) => track.id)
    .join(",");
  const load = useCallback(async () => {
    const list = ids ? ids.split(",") : [];
    const counts = await Promise.all(
      list.map((trackId) =>
        loadOpenQuestions(workspaceId, projectId, trackId).then(
          (questions) => questions.filter((question) => question.asked_by.kind !== "human").length,
          () => 0,
        ),
      ),
    );
    return new Map(list.map((trackId, at) => [trackId, counts[at]]));
  }, [workspaceId, projectId, ids]);
  const { value, reload } = useReloading(load);
  useProjectEvents(projectId, reload);
  return value;
}
