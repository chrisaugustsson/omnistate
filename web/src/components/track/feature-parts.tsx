import { useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { ArrowRight, Box, FileText, Palette, PenTool } from "lucide-react";
import { ApiError } from "@/api/client";
import { linkTrackFeature } from "@/api/features";
import type { HomeDoc, TrackSummary } from "@/api/generated";
import { createTrack } from "@/api/parts";
import { Avatar } from "@/components/shell/avatar";
import { PartIcon } from "@/components/shell/part-icon";
import { PanelSection, PropertiesPanel, PropertyRow } from "@/components/shell/properties-panel";
import { StateIcon, stateLabel } from "@/components/shell/state-icon";
import { cn } from "@/lib/utils";
import { foldReplaced } from "./decision-log";
import { useFeature, type FeatureData } from "./feature-data";
import { nameOf } from "./people";
import { Dialog, Progress, buttonClass, chipClass, shortDate } from "./ui";

// The pieces of a feature's pages: its mark, the "Start track" action, the
// banner for a track that changes it now, and its properties panel.

/** A feature's mark beside its title: the cube in a hairline square. */
export function FeatureMark() {
  return (
    <span aria-hidden className="grid size-7 shrink-0 place-items-center rounded-control border border-line bg-sheet text-ink-2">
      <Box strokeWidth={2.25} className="size-(--icon)" />
    </span>
  );
}

/** The first sentence or line of a description. */
export function firstLine(text: string): string {
  const line = text.split(/\r?\n/, 1)[0]?.trim() ?? "";
  const sentence = line.match(/^.*?[.!?](\s|$)/)?.[0]?.trim();
  return sentence ?? line;
}

const field =
  "w-full rounded-control border border-line-strong bg-sheet px-2.5 py-1.5 text-ink outline-none placeholder:text-ink-3 focus-visible:border-brand focus-visible:shadow-focus";

/** "Start track": a new track on the feature's part that changes the feature, and to its page. */
export function StartTrack({ data }: { data: FeatureData }) {
  const { workspaceId, projectId, partId, featureId, feature } = data;
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();
  const close = () => {
    setOpen(false);
    setError(null);
  };

  async function start() {
    setBusy(true);
    setError(null);
    try {
      const track = await createTrack(workspaceId, projectId, { name: name.trim(), description: description.trim(), part_id: partId });
      await linkTrackFeature(workspaceId, projectId, track.id, featureId);
      setOpen(false);
      await navigate({ to: "/w/$workspaceId/projects/$projectId/tracks/$trackId", params: { workspaceId, projectId, trackId: track.id } });
    } catch (failure) {
      setError(
        failure instanceof ApiError && failure.code === "name_taken"
          ? "Another track of the project has this name."
          : "The track could not be started. Try again.",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <button type="button" className={buttonClass({ variant: "primary" })} onClick={() => setOpen(true)}>
        Start track
      </button>
      {open && (
        <Dialog
          title={`Start a track on ${feature?.name ?? "the feature"}`}
          onClose={close}
          actions={
            <>
              <button type="button" className={buttonClass()} onClick={close}>
                Cancel
              </button>
              <button type="button" className={buttonClass({ variant: "primary" })} disabled={!name.trim() || busy} onClick={() => void start()}>
                Start track
              </button>
            </>
          }
        >
          <p>A track is one change to the feature: grilled, designed, built and merged. You own it.</p>
          <div className="mt-3 space-y-2">
            <input aria-label="Track name" placeholder="Name" value={name} onChange={(event) => setName(event.target.value)} className={cn(field, "font-medium")} />
            <textarea
              aria-label="Description"
              placeholder="What changes, in a sentence"
              rows={2}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              className={cn(field, "resize-y")}
            />
            {error && (
              <p role="alert" className="text-danger-ink">
                {error}
              </p>
            )}
          </div>
        </Dialog>
      )}
    </>
  );
}

/** The quiet row under the lede for each track in flight that changes the feature: its name, what it changes, its progress and owner. */
export function BeingChanged({ data }: { data: FeatureData }) {
  const { changing, workspaceId, projectId, people } = data;
  if (changing.length === 0) return null;
  return (
    <div className="mt-5 space-y-2 min-[960px]:max-w-[calc(100%-var(--panel-w)-3rem)]">
      {changing.map((summary) => {
        const { track } = summary;
        const owner = nameOf(people, track.owner_user_id);
        const what = firstLine(track.description);
        return (
          <Link
            key={track.id}
            to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
            params={{ workspaceId, projectId, trackId: track.id }}
            className="group flex min-h-9 items-center gap-3 rounded-control bg-brand-tint py-1.5 pr-2 pl-3 text-ink transition-colors hover:bg-[color-mix(in_oklab,var(--brand-tint),var(--brand)_6%)]"
          >
            <StateIcon state={track.state} />
            <p className="min-w-0 flex-1">
              Being changed by <b className="font-semibold">{track.name}</b>
              {what && `: ${what.replace(/\.$/, "")}`}. Not on main yet.
            </p>
            {summary.stories && <Progress done={summary.stories.done} total={summary.stories.total} />}
            <Avatar name={owner} tintKey={track.owner_user_id ?? undefined} title={owner} />
            <ArrowRight aria-hidden strokeWidth={2.25} className="size-(--icon-sm) shrink-0 text-brand transition-transform group-hover:translate-x-0.5" />
          </Link>
        );
      })}
    </div>
  );
}

/** How a track stands to the feature, for the panel and the Tracks tab: "Changing it · Planning · Christopher". */
export function trackFacts(summary: TrackSummary, owner: string): string {
  const { state } = summary.track;
  if (state === "done") return `Changed it · Done${summary.last_activity ? ` ${shortDate(summary.last_activity)}` : ""} · ${owner}`;
  if (state === "dropped") return `Dropped · ${owner}`;
  return `Changing it · ${stateLabel(state)} · ${owner}`;
}

export function DocGlyph({ kind, className }: { kind: string; className?: string }) {
  const Icon = kind === "design" ? PenTool : kind === "design-system" ? Palette : FileText;
  return <Icon aria-hidden strokeWidth={2.25} className={cn("size-(--icon-sm) shrink-0 text-ink-2", className)} />;
}

const PILLAR_NAMES: Record<string, string> = { spec: "spec", ui: "UI", architecture: "architecture", db: "database", research: "research" };

/** The feature's properties: its part, its decisions, the tracks that changed it and its docs. */
export function FeaturePanel({ className }: { className?: string }) {
  const { workspaceId, projectId, partId, part, tone, decisions, tracks, docs, people } = useFeature();
  const params = { workspaceId, projectId };
  const folded = decisions ? foldReplaced([...decisions.decisions, ...decisions.earlier], decisions.decisions) : null;
  return (
    <PropertiesPanel className={className}>
      <PanelSection>
        <PropertyRow label="Part">
          <Link to="/w/$workspaceId/projects/$projectId/parts/$partId" params={{ ...params, partId }} className={chipClass}>
            {part && <PartIcon name={part.name} kind={part.kind} tone={tone} size="sm" />}
            <span className="truncate">{part?.name}</span>
          </Link>
          {part && <span className="text-meta text-ink-3">{part.kind}</span>}
        </PropertyRow>
        <PropertyRow label="Decisions">
          {folded && (
            <span className="px-1 leading-6">
              <span className="font-medium text-ink">{folded.top.length} current</span>
              {folded.folded > 0 && <span className="text-ink-3"> · {folded.folded} replaced</span>}
            </span>
          )}
        </PropertyRow>
      </PanelSection>
      <PanelSection title="Tracks">
        {tracks && tracks.length === 0 && <p className="px-1 text-meta text-ink-3">No track has changed it yet.</p>}
        {tracks?.map((summary) => (
          <Link
            key={summary.track.id}
            to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
            params={{ ...params, trackId: summary.track.id }}
            className="grid grid-cols-[var(--icon-sm)_minmax(0,1fr)] gap-x-2 rounded-control px-1 py-1.5 transition-colors duration-(--t-fast) hover:bg-hover"
          >
            <span className="mt-[3px] grid">
              <StateIcon state={summary.track.state} />
            </span>
            <span className="truncate font-medium text-ink">{summary.track.name}</span>
            <span className="col-start-2 truncate text-meta text-ink-3">{trackFacts(summary, nameOf(people, summary.track.owner_user_id))}</span>
          </Link>
        ))}
      </PanelSection>
      <PanelSection title="Docs">
        {docs.length === 0 && <p className="px-1 text-meta text-ink-3">No docs on it yet.</p>}
        {docs.map((doc: HomeDoc) => (
          <Link
            key={doc.id}
            to="/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/docs/$docId"
            params={{ ...params, partId, featureId: doc.feature_id ?? "", docId: doc.id }}
            className="flex h-[30px] items-center gap-2 rounded-control px-1 font-medium text-ink transition-colors duration-(--t-fast) hover:bg-hover"
          >
            <DocGlyph kind={doc.kind} />
            <span className="min-w-0 truncate">{doc.title}</span>
            <span className="ml-auto pl-2 text-meta font-normal text-ink-3">{PILLAR_NAMES[doc.pillar] ?? doc.pillar}</span>
          </Link>
        ))}
      </PanelSection>
    </PropertiesPanel>
  );
}
