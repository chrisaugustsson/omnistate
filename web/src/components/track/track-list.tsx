import { Link } from "@tanstack/react-router";
import type { TrackSummary } from "@/api/generated";
import { Avatar } from "@/components/shell/avatar";
import { StateIcon, TRACK_STATES, stateLabel } from "@/components/shell/state-icon";
import { timeAgo } from "@/lib/format";
import { firstLine } from "./feature-parts";
import { nameOf, type People } from "./people";
import { Progress } from "./ui";

// The tracks of a feature as its Tracks tab lists them: grouped by state
// (Planning, Building, Done, Dropped) under a band each, every row the track's
// state, name and what it changes, its progress, owner and last change.

function Row({ summary, params, people }: { summary: TrackSummary; params: { workspaceId: string; projectId: string }; people: People | null }) {
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
        className="flex items-start gap-3 border-t border-line px-3 py-2.5 transition-colors duration-(--t-fast) hover:bg-frame"
      >
        <span className="grid h-5 w-(--avatar) shrink-0 place-items-center">
          <StateIcon state={track.state} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium text-ink">{track.name}</span>
          <span className="block truncate text-ink-3">
            {verb}
            {what ? `: ${what}` : ""}
          </span>
        </span>
        <span className="flex h-5 shrink-0 items-center gap-3">
          {summary.stories && summary.stories.total > 0 && <Progress done={summary.stories.done} total={summary.stories.total} />}
          <Avatar name={owner} tintKey={track.owner_user_id ?? undefined} title={owner} />
          <span title={new Date(when).toLocaleString()} className="w-16 text-right text-meta whitespace-nowrap text-ink-3">
            {timeAgo(when)}
          </span>
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
  return (
    <div className="overflow-hidden rounded-card border border-line">
      {groups.map((group, at) => (
        <section key={group.state} aria-label={stateLabel(group.state)} className={at > 0 ? "border-t border-line" : undefined}>
          <h2 className="flex h-(--band-h) items-center gap-2 bg-band px-3 font-medium text-ink">
            <span className="grid w-(--avatar) place-items-center">
              <StateIcon state={group.state} />
            </span>
            {stateLabel(group.state)}
            <span className="font-medium text-ink-3 tabular-nums">{group.tracks.length}</span>
          </h2>
          <ul>
            {group.tracks.map((summary) => (
              <Row key={summary.track.id} summary={summary} params={params} people={people} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
