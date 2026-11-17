import { createContext, useCallback, useContext, useMemo } from "react";
import { loadFeatureDecisions, type LevelDecisions } from "@/api/decisions";
import { loadFeature, loadFeatureTracks } from "@/api/features";
import type { FeatureBody, HomeDoc, HomeResponse, PartBody, TrackSummary } from "@/api/generated";
import { inFlight } from "@/components/shell/state-icon";
import { partTone } from "@/lib/identity";
import { useReloading } from "@/lib/reloading";
import { useHome } from "@/lib/use-home";
import { useParts } from "@/lib/use-parts";
import { useProjectEvents } from "@/live/context";
import { usePeople, type People } from "./people";

// What a feature's pages know about the feature, loaded once by its layout and
// shared with its tabs: the feature and its part, the tracks that changed it or
// change it now, its docs and its decisions.

export type FeatureData = {
  workspaceId: string;
  projectId: string;
  partId: string;
  featureId: string;
  feature: FeatureBody | null;
  /** The feature could not be loaded: the part has no such feature, or it was deleted. */
  failed: boolean;
  home: HomeResponse | null;
  parts: PartBody[] | null;
  part: PartBody | undefined;
  tone: number;
  people: People | null;
  /** The tracks linked to the feature, newest activity first; null while they load. */
  tracks: TrackSummary[] | null;
  /** The tracks in flight (planning or building) that change it now. */
  changing: TrackSummary[];
  docs: HomeDoc[];
  /** Its design docs: those on the UI pillar or drawn on a canvas. */
  designDocs: HomeDoc[];
  decisions: LevelDecisions | null;
};

const FeatureContext = createContext<FeatureData | null>(null);

export const FeatureProvider = FeatureContext.Provider;

/** The feature's data, inside the feature's layout. */
export function useFeature(): FeatureData {
  const data = useContext(FeatureContext);
  if (!data) throw new Error("useFeature is used outside a feature's layout");
  return data;
}

const recent = (summary: TrackSummary) => Date.parse(summary.last_activity ?? summary.track.created_at) || 0;

/** Loads what the feature's pages show; the feature's layout calls it and provides it to the tabs. */
export function useFeatureData(workspaceId: string, projectId: string, partId: string, featureId: string): FeatureData {
  const loadOne = useCallback(() => loadFeature(workspaceId, projectId, partId, featureId), [workspaceId, projectId, partId, featureId]);
  const one = useReloading(loadOne);
  const loadTracks = useCallback(() => loadFeatureTracks(workspaceId, projectId, partId, featureId), [workspaceId, projectId, partId, featureId]);
  const linked = useReloading(loadTracks);
  const loadDecisions = useCallback(
    () => loadFeatureDecisions(workspaceId, projectId, partId, featureId),
    [workspaceId, projectId, partId, featureId],
  );
  const decisions = useReloading(loadDecisions);
  const { home } = useHome(workspaceId, projectId);
  const { parts } = useParts(workspaceId, projectId);
  const people = usePeople(workspaceId, projectId);
  useProjectEvents(projectId, (event) => {
    const name = event.event;
    if (name.startsWith("feature.")) one.reload();
    if (name.startsWith("track.")) linked.reload();
    if (name === "decision.recorded") decisions.reload();
  });

  const tracks = useMemo(() => {
    if (!linked.value) return null;
    const summaries = new Map((home?.tracks ?? []).map((summary) => [summary.track.id, summary]));
    return linked.value.map((track) => summaries.get(track.id) ?? { track, docs: 0, waiting: 0 }).sort((a, b) => recent(b) - recent(a));
  }, [linked.value, home]);
  const docs = useMemo(() => (home?.docs ?? []).filter((doc) => doc.feature_id === featureId), [home, featureId]);

  return {
    workspaceId,
    projectId,
    partId,
    featureId,
    feature: one.value,
    failed: one.failed && !one.value,
    home,
    parts,
    part: parts?.find((candidate) => candidate.id === partId),
    tone: partTone(parts, partId),
    people,
    tracks,
    changing: (tracks ?? []).filter((summary) => inFlight(summary.track.state)),
    docs,
    designDocs: docs.filter((doc) => doc.pillar === "ui" || doc.kind === "design" || doc.kind === "design-system"),
    decisions: decisions.value,
  };
}
