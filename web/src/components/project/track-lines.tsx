import { Link } from "@tanstack/react-router";
import type { PartBody, TrackSummary } from "@/api/generated";
import { Avatar } from "@/components/shell/avatar";
import { PartIcon } from "@/components/shell/part-icon";
import { StateIcon, stateLabel } from "@/components/shell/state-icon";
import { partTone } from "@/lib/identity";
import { cn } from "@/lib/utils";
import type { ProjectMap } from "./project-data";
import { Badge, Progress, RowLead, rowClass, shortWhen } from "./rows";

// A track as a row: in an Overview's In flight (two lines: the name over what
// it changes and where it split) and in a Tracks list (one line). Both show
// its state icon, its open questions, its part (on the project's pages), its
// progress and its owner.

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
    <PartChip
      className="w-[104px]"
      name={part?.name ?? summary.part_name ?? "Part"}
      kind={part?.kind ?? "service"}
      tone={partTone(parts, summary.track.part_id)}
      deleted={summary.part_deleted}
    />
  );
}

/** The owner of a track as an avatar; nothing for a track nobody owns. */
export function OwnerAvatar({ userId, names }: { userId?: string | null; names: ReadonlyMap<string, string> | null }) {
  if (!userId) return <span aria-hidden className="w-(--avatar) shrink-0" />;
  const name = names?.get(userId) ?? "Owner";
  return (
    <span className="inline-grid shrink-0" title={`Owned by ${name}`}>
      <Avatar name={name} tintKey={userId} />
      <span className="sr-only">Owned by {name}</span>
    </span>
  );
}

function Questions({ count }: { count: number | undefined }) {
  if (!count) return null;
  return <Badge tone="brand">{count === 1 ? "1 question" : `${count} questions`}</Badge>;
}

type RowProps = {
  summary: TrackSummary;
  all: readonly TrackSummary[];
  params: Params;
  map: ProjectMap | null;
  names: ReadonlyMap<string, string> | null;
  questions: ReadonlyMap<string, number> | null | undefined;
  /** Show the track's part (on the project's pages). */
  showPart: boolean;
};

/** A track in an Overview's In flight: its name over what it changes, then questions, part, progress and owner. */
export function FlightRow({ summary, all, params, map, names, questions, showPart }: RowProps) {
  const { track } = summary;
  const stories = summary.stories ?? { total: 0, open: 0, done: 0 };
  const facts = trackFacts(summary, map, all);
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
        params={{ ...params, trackId: track.id }}
        className={rowClass({ two: true, link: true })}
      >
        <RowLead two>
          <StateIcon state={track.state} label={stateLabel(track.state)} />
        </RowLead>
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="truncate font-medium">{track.name}</span>
          {facts && <span className="truncate text-ink-3">{facts}</span>}
        </span>
        <span className="flex min-h-5 shrink-0 items-center gap-3">
          <Questions count={questions?.get(track.id)} />
          {showPart && <TrackPart summary={summary} parts={map?.parts ?? null} />}
          <Progress done={stories.done} total={stories.total} className="w-24" />
          <OwnerAvatar userId={track.owner_user_id} names={names} />
        </span>
      </Link>
    </li>
  );
}

/** A track in a Tracks list: state, name, what it changes, then questions, part, progress, owner and when it last moved. */
export function TrackLine({ summary, all, params, map, names, questions, showPart }: RowProps) {
  const { track, last_activity } = summary;
  const stories = summary.stories ?? { total: 0, open: 0, done: 0 };
  const facts = trackFacts(summary, map, all);
  const when = last_activity ?? track.created_at;
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
        params={{ ...params, trackId: track.id }}
        className={rowClass({ link: true, inset: true })}
      >
        <RowLead>
          <StateIcon state={track.state} label={stateLabel(track.state)} />
        </RowLead>
        <span className="flex min-w-0 flex-1 items-center gap-2">
          <span className={cn("shrink-0 truncate font-medium", track.state === "dropped" && "text-ink-2")}>{track.name}</span>
          {facts && <span className="min-w-0 flex-1 truncate text-ink-3">{facts}</span>}
        </span>
        <span className="flex shrink-0 items-center gap-3">
          <Questions count={questions?.get(track.id)} />
          {showPart && <TrackPart summary={summary} parts={map?.parts ?? null} />}
          <Progress done={stories.done} total={stories.total} className="w-24" />
          <OwnerAvatar userId={track.owner_user_id} names={names} />
          <span
            title={last_activity ? `Last activity ${new Date(last_activity).toLocaleString()}` : `Created ${new Date(track.created_at).toLocaleString()}`}
            className="w-14 text-right text-meta whitespace-nowrap text-ink-3"
          >
            {shortWhen(when)}
          </span>
        </span>
      </Link>
    </li>
  );
}
