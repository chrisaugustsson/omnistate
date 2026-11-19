import { Link } from "@tanstack/react-router";
import type { PartBody, TrackSummary } from "@/api/generated";
import { PartIcon } from "@/components/shell/part-icon";
import { StateIcon, stateLabel } from "@/components/shell/state-icon";
import { partTone } from "@/lib/identity";
import { cn } from "@/lib/utils";
import { Flags, Slot } from "@/components/home/rows";
import type { TrackFlags } from "@/components/home/use-home-data";
import type { ProjectMap } from "./project-data";
import { RowLead, rowClass, shortWhen } from "./rows";

// A track as a row of a Tracks list: its state icon, its name and what it
// changes, then one signal at most (questions waiting, else blocked), its part
// (on the project's pages) and when it last moved.

type Params = { workspaceId: string; projectId: string };

/** What a track changes and how it split, as the grey line under or beside its name. */
export function trackFacts(summary: TrackSummary, map: ProjectMap | null, all: readonly TrackSummary[]): string {
  const { track } = summary;
  const facts: string[] = [];
  const features = map?.changes.get(track.id) ?? [];
  const finished = track.state === "done" || track.state === "dropped";
  if (map) {
    const names = features.map((feature) => feature.name).join(", ");
    facts.push(features.length > 0 ? `${finished ? "changed" : "changes"} ${names}` : `${finished ? "changed" : "changes"} no feature`);
  }
  const from = track.split_from_track_id ? all.find((other) => other.track.id === track.split_from_track_id) : undefined;
  if (from) facts.push(`split off from ${from.track.name}`);
  const to = all.filter((other) => other.track.split_from_track_id === track.id);
  if (to.length > 0) facts.push(`split off to ${to.map((other) => other.track.name).join(", ")}`);
  return facts.join(" · ");
}

/** A part as a chip: its icon in its colour and its name. */
export function PartChip({ name, kind, tone, deleted, className }: { name: string; kind: string; tone: number; deleted?: boolean; className?: string }) {
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1.5 font-medium whitespace-nowrap text-ink-2", className)}>
      <PartIcon name={name} kind={kind} tone={tone} size="sm" />
      <span className="truncate">
        {name}
        {deleted && <span className="font-normal text-ink-3"> (deleted)</span>}
      </span>
    </span>
  );
}

/** The part of a track as a chip; a deleted part by its name, marked. */
function TrackPart({ summary, parts }: { summary: TrackSummary; parts: readonly PartBody[] | null }) {
  const part = parts?.find((candidate) => candidate.id === summary.track.part_id);
  return (
    <Slot kind="part">
      <PartChip
        className="w-full"
        name={part?.name ?? summary.part_name ?? "Part"}
        kind={part?.kind ?? "service"}
        tone={partTone(parts, summary.track.part_id)}
        deleted={summary.part_deleted}
      />
    </Slot>
  );
}

type RowProps = {
  summary: TrackSummary;
  all: readonly TrackSummary[];
  params: Params;
  map: ProjectMap | null;
  /** Each track's questions and blocked mark (useRowFlags). */
  questions: ReadonlyMap<string, TrackFlags> | null | undefined;
  /** Show the track's part (on the project's pages). */
  showPart: boolean;
};

/**
 * A track in a Tracks list: state, name, what it changes, then one signal at
 * most (its open questions, else a blocked mark), its part on the project's
 * pages, and when it last moved. Progress, threads and owner live on the
 * track's own page. On the phone the facts go to a second line under the name.
 */
export function TrackLine({ summary, all, params, map, questions, showPart }: RowProps) {
  const { track, last_activity } = summary;
  const facts = trackFacts(summary, map, all);
  const when = last_activity ?? track.created_at;
  const flags = questions?.get(track.id);
  const signal = flags && (flags.questions > 0 ? { questions: flags.questions, blocked: null } : flags);
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
        params={{ ...params, trackId: track.id }}
        className={cn(rowClass({ link: true, inset: true }), "max-md:flex-wrap max-md:items-start max-md:gap-y-1 max-md:py-2.5")}
      >
        <RowLead>
          <StateIcon state={track.state} label={stateLabel(track.state)} />
        </RowLead>
        <span className="flex min-w-0 flex-1 items-center gap-2 max-md:basis-[calc(100%-var(--avatar)-12px)]">
          <span className={cn("max-w-full min-w-0 shrink-0 truncate font-medium", track.state === "dropped" && "text-ink-2")}>{track.name}</span>
          {facts && <span className="min-w-0 flex-1 truncate text-ink-3">{facts}</span>}
        </span>
        <span className="flex shrink-0 items-center gap-3 max-md:min-w-0 max-md:basis-full max-md:flex-wrap max-md:gap-x-2.5 max-md:gap-y-1 max-md:pl-8 max-md:*:w-auto max-md:*:empty:hidden">
          <Flags flags={signal} slots />
          {showPart && <TrackPart summary={summary} parts={map?.parts ?? null} />}
          <Slot kind="when">
            <span
              title={last_activity ? `Last activity ${new Date(last_activity).toLocaleString()}` : `Created ${new Date(track.created_at).toLocaleString()}`}
              className="text-meta whitespace-nowrap text-ink-3"
            >
              {shortWhen(when)}
            </span>
          </Slot>
        </span>
      </Link>
    </li>
  );
}
