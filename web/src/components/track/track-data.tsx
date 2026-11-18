import { createContext, useCallback, useContext, useMemo, useState } from "react";
import { loadOpenQuestions } from "@/api/behaviour";
import { loadTrackDecisions } from "@/api/decisions";
import { loadHeadOutline } from "@/api/docs";
import { loadTrackFeatures } from "@/api/features";
import type { DecisionBody, FeatureBody, HomeDoc, HomeResponse, HomeThread, LinkedBody, PartBody, TrackSummary } from "@/api/generated";
import { loadStories } from "@/api/stories";
import { partTone } from "@/lib/identity";
import { useReloading } from "@/lib/reloading";
import { useHome } from "@/lib/use-home";
import { useParts } from "@/lib/use-parts";
import { useProjectEvents } from "@/live/context";
import { nameOf, usePeople, type People } from "./people";

// What a track's pages know about the track, loaded once by the track's layout
// and shared with its tabs: the track and its part, its people and what the
// reader may do, the features it changes, its stories' links to other tracks,
// its open behaviour questions, decisions, docs by pillar and threads. Each part
// is loaded again on the live events that concern it.

/** The docs of each of a track's tabs, by pillar: Spec (and research), Design (UI), How (architecture and tables). */
export const TAB_PILLARS = {
  spec: ["spec", "research"],
  design: ["ui"],
  how: ["architecture", "db"],
} as const;

export type DocTab = keyof typeof TAB_PILLARS;

export type Move = { to: string; label: string; danger?: boolean; final?: boolean; ownerOnly?: boolean };

/** The moves out of each state, the usual next one first (US-311). Only the owner marks a track done; done and dropped are final. */
const MOVES: Record<string, Move[]> = {
  planning: [
    { to: "building", label: "Start building" },
    { to: "dropped", label: "Drop track", danger: true, final: true },
  ],
  building: [
    { to: "done", label: "Mark done", ownerOnly: true, final: true },
    { to: "planning", label: "Back to planning" },
    { to: "dropped", label: "Drop track", danger: true, final: true },
  ],
};

export type MoveChoice = Move & { allowed: boolean; reason?: string; hint?: string };

/** Every move the track's state allows, each saying whether the reader may make it and, if not, why. */
export function movesFor(state: string, owner: string, may: { owner: boolean; admin: boolean }): MoveChoice[] {
  return (MOVES[state] ?? []).map((move) => {
    if (may.owner) return { ...move, allowed: true };
    if (may.admin && !move.ownerOnly) return { ...move, allowed: true };
    if (may.admin) return { ...move, allowed: false, reason: `Only ${owner} marks it done`, hint: "Owner only" };
    return { ...move, allowed: false, reason: `Only ${owner} or an admin moves this track`, hint: "Owner or admin" };
  });
}

export type TrackData = {
  workspaceId: string;
  projectId: string;
  trackId: string;
  home: HomeResponse | null;
  /** The track, once the home is loaded; undefined for a track the project does not have. */
  summary: TrackSummary | undefined;
  /** The home is loaded and has no such track. */
  missing: boolean;
  failed: boolean;
  parts: PartBody[] | null;
  part: PartBody | undefined;
  tone: number;
  /** Whether the track is on a service part, where the Design tab only shows when it has design docs. */
  onService: boolean;
  people: People | null;
  ownerName: string;
  isOwner: boolean;
  /** The reader may hand the track over: its owner or an admin. */
  mayHandOver: boolean;
  /** The reader may record decisions on it: its owner. */
  mayRecord: boolean;
  moves: MoveChoice[];
  features: FeatureBody[] | null;
  linked: LinkedBody[];
  /** How many behaviour questions an agent asked that wait for an answer. */
  openQuestions: number;
  /** The Behaviour screen, which reads the questions itself, says how many wait while it is open. */
  reportOpenQuestions: (count: number) => void;
  decisions: DecisionBody[] | null;
  reloadDecisions: () => void;
  docs: Record<DocTab, HomeDoc[]>;
  /** How many boards the track's design docs hold; undefined until they are counted. */
  boards: number | undefined;
  threads: HomeThread[];
  /** Every track of the project, for the split links. */
  tracks: TrackSummary[];
};

const TrackContext = createContext<TrackData | null>(null);

export const TrackProvider = TrackContext.Provider;

/** The track's data, inside the track's layout. */
export function useTrack(): TrackData {
  const data = useContext(TrackContext);
  if (!data) throw new Error("useTrack is used outside a track's layout");
  return data;
}

/**
 * Loads what the track's pages show; the track's layout calls it and provides
 * it to the tabs. While the Behaviour tab is open (`onBehaviour`) its screen
 * reads the open questions and reports their count, so they are not read twice.
 */
export function useTrackData(workspaceId: string, projectId: string, trackId: string, onBehaviour = false): TrackData {
  const { home, failed } = useHome(workspaceId, projectId);
  const { parts } = useParts(workspaceId, projectId);
  const people = usePeople(workspaceId, projectId);
  const summary = home?.tracks?.find((candidate) => candidate.track.id === trackId);
  const track = summary?.track;

  const loadFeatures = useCallback(() => loadTrackFeatures(workspaceId, projectId, trackId), [workspaceId, projectId, trackId]);
  const features = useReloading(loadFeatures, `track-features:${trackId}`);
  const loadLinks = useCallback(() => loadStories(workspaceId, projectId, trackId), [workspaceId, projectId, trackId]);
  const stories = useReloading(loadLinks, `stories:${trackId}`);
  const loadQuestions = useCallback(
    () => (onBehaviour ? Promise.resolve(null) : loadOpenQuestions(workspaceId, projectId, trackId).catch(() => [])),
    [workspaceId, projectId, trackId, onBehaviour],
  );
  const questions = useReloading(loadQuestions, onBehaviour ? undefined : `questions:${trackId}`);
  const [reported, setReported] = useState<number | null>(null);
  const loadDecisions = useCallback(() => loadTrackDecisions(workspaceId, projectId, trackId), [workspaceId, projectId, trackId]);
  const decisions = useReloading(loadDecisions, `track-decisions:${trackId}`);

  const docs = useMemo(() => {
    const own = (home?.docs ?? []).filter((doc) => doc.track_id === trackId);
    const of = (pillars: readonly string[]) => own.filter((doc) => pillars.includes(doc.pillar));
    return { spec: of(TAB_PILLARS.spec), design: of(TAB_PILLARS.design), how: of(TAB_PILLARS.how) };
  }, [home, trackId]);
  const designIds = docs.design.filter((doc) => doc.kind === "design").map((doc) => doc.id).join(",");
  const loadBoards = useCallback(async () => {
    const ids = designIds ? designIds.split(",") : [];
    const outlines = await Promise.all(ids.map((id) => loadHeadOutline(workspaceId, projectId, id).catch(() => ({ boards: [] }))));
    return outlines.reduce((sum, outline) => sum + outline.boards.length, 0);
  }, [workspaceId, projectId, designIds]);
  const boards = useReloading(loadBoards, `track-boards:${trackId}:${designIds}`);

  useProjectEvents(projectId, (event) => {
    const name = event.event;
    if (name.startsWith("track.")) features.reload();
    if (name.startsWith("story.")) stories.reload();
    if (name === "behaviour.question_changed" && event.data.track_id === trackId && !onBehaviour) questions.reload();
    if (name === "decision.recorded") decisions.reload();
    if (name === "doc.changed" || name === "doc.created" || name === "doc.deleted") boards.reload();
  });

  const part = parts?.find((candidate) => candidate.id === track?.part_id);
  const ownerName = nameOf(people, track?.owner_user_id);
  const isOwner = !!people?.me && people.me === track?.owner_user_id;
  const admin = people?.admin ?? false;
  // The count from the Behaviour screen while it is open, else from the questions read here. While the
  // one in use has not arrived yet (right after moving to or from Behaviour), the last count of the
  // track stands, so the sidebar does not drop it for a frame.
  const counted = questions.value ? questions.value.filter((question) => question.asked_by.kind !== "human").length : null;
  const known = onBehaviour ? reported : counted;
  const [last, setLast] = useState<{ trackId: string; count: number } | null>(null);
  if (known !== null && (last?.trackId !== trackId || last.count !== known)) setLast({ trackId, count: known });
  if (!onBehaviour && reported !== null) setReported(null);
  const openQuestions = known ?? (last?.trackId === trackId ? last.count : 0);

  return {
    workspaceId,
    projectId,
    trackId,
    home,
    summary,
    missing: home !== null && !summary,
    failed,
    parts,
    part,
    tone: partTone(parts, track?.part_id ?? ""),
    onService: part?.kind === "service",
    people,
    ownerName,
    isOwner,
    mayHandOver: isOwner || admin,
    mayRecord: isOwner,
    moves: track && people ? movesFor(track.state, ownerName, { owner: isOwner, admin }) : [],
    features: features.value,
    linked: stories.value?.linked ?? [],
    openQuestions,
    reportOpenQuestions: setReported,
    decisions: decisions.value,
    reloadDecisions: decisions.reload,
    docs,
    boards: designIds ? (boards.value ?? undefined) : 0,
    threads: (home?.threads ?? []).filter((thread) => thread.track_id === trackId),
    tracks: home?.tracks ?? [],
  };
}
