import { useMemo } from "react";
import { Link } from "@tanstack/react-router";
import type { TrackSummary } from "@/api/generated";
import { Flags, Slot } from "@/components/home/rows";
import { useTrackFlags, type TrackFlags } from "@/components/home/use-home-data";
import { Band, rowClass } from "@/components/project/rows";
import { Avatar } from "@/components/shell/avatar";
import { StateIcon, TRACK_STATES, stateLabel } from "@/components/shell/state-icon";
import { timeAgo } from "@/lib/format";
import { cn } from "@/lib/utils";
import { firstLine } from "./feature-parts";
import { nameOf, type People } from "./people";
import { Progress } from "./ui";

// The tracks of a feature as its Tracks tab lists them, like a part's Tracks:
// grouped by state (Planning, Building, Done, Dropped) under a band each, every
// row the track's state, name and what it changes, its questions and blocked
// mark, progress, owner and last change; on the phone those facts take a third
// line under what it changes.

function Row({
  summary,
  params,
  people,
  flags,
}: {
  summary: TrackSummary;
  params: { workspaceId: string; projectId: string };
  people: People | null;
  flags: TrackFlags | undefined;
}) {
  const { track } = summary;
  const owner = nameOf(people, track.owner_user_id);
  const verb = track.state === "done" ? "Changed it" : track.state === "dropped" ? "Was to change it" : "Changing it";
  const what = firstLine(track.description);
  const when = summary.last_activity ?? track.created_at;
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
        params={{ ...params, trackId: track.id }}
        className={cn(rowClass({ two: true, link: true, inset: true }), "grid grid-cols-[var(--avatar)_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-0.5")}
      >
        <span className="grid h-5 w-(--avatar) shrink-0 place-items-center">
          <StateIcon state={track.state} />
        </span>
        <span className="min-w-0 truncate font-medium text-ink">{track.name}</span>
        <span className="col-span-2 col-start-2 row-start-2 min-w-0 truncate text-ink-3">
          {verb}
          {what ? `: ${what}` : ""}
        </span>
        <span className="col-start-3 row-start-1 flex h-5 shrink-0 items-center gap-3 max-md:col-span-2 max-md:col-start-2 max-md:row-start-3 max-md:mt-0.5 max-md:h-auto max-md:flex-wrap max-md:gap-y-1 max-md:gap-x-2.5 max-md:*:w-auto max-md:*:empty:hidden">
          <Flags flags={flags} slots />
          <Slot kind="progress">
            {summary.stories && summary.stories.total > 0 && <Progress done={summary.stories.done} total={summary.stories.total} />}
          </Slot>
          <Slot kind="owner">
            <Avatar name={owner} tintKey={track.owner_user_id ?? undefined} title={owner} />
          </Slot>
          <Slot kind="when">
            <span title={new Date(when).toLocaleString()} className="text-meta whitespace-nowrap text-ink-3">
              {timeAgo(when)}
            </span>
          </Slot>
        </span>
      </Link>
    </li>
  );
}

export function TracksByState({
  tracks,
  params,
  people,
}: {
  tracks: TrackSummary[];
  params: { workspaceId: string; projectId: string };
  people: People | null;
}) {
  const groups = TRACK_STATES.map((state) => ({ state, tracks: tracks.filter((summary) => summary.track.state === state) })).filter(
    (group) => group.tracks.length > 0,
  );
  const bodies = useMemo(() => tracks.map(({ track }) => track), [tracks]);
  const flags = useTrackFlags(params.workspaceId, bodies);
  return (
    <div>
      {groups.map((group) => (
        <section key={group.state} aria-label={stateLabel(group.state)} className="mb-2">
          <h2>
            <Band icon={<StateIcon state={group.state} />} count={group.tracks.length}>
              {stateLabel(group.state)}
            </Band>
          </h2>
          <ul className="border-b border-line [&>li:first-child>a]:border-t-0">
            {group.tracks.map((summary) => (
              <Row key={summary.track.id} summary={summary} params={params} people={people} flags={flags.get(summary.track.id)} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
