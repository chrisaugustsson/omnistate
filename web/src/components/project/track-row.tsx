import { Link } from "@tanstack/react-router";
import type { TrackSummary } from "@/api/generated";
import { rowClass } from "@/components/project/items";
import { StateLabel } from "@/components/project/new-track";
import { timeAgo } from "@/lib/format";

const FINISHED = new Set(["done", "dropped"]);

/** A track's last activity, or its creation while it has none. */
const updatedAt = ({ track, last_activity }: TrackSummary) => Date.parse(last_activity ?? track.created_at);

const byRecent = (a: TrackSummary, b: TrackSummary) =>
  updatedAt(b) - updatedAt(a) || a.track.name.toLowerCase().localeCompare(b.track.name.toLowerCase());

/** Active tracks first, then done and dropped, each group with the most recently updated first. */
export function orderTracks(tracks: TrackSummary[]): TrackSummary[] {
  return [
    ...tracks.filter((summary) => !FINISHED.has(summary.track.state)).sort(byRecent),
    ...tracks.filter((summary) => FINISHED.has(summary.track.state)).sort(byRecent),
  ];
}

export function TrackRow({ summary, params }: { summary: TrackSummary; params: { workspaceId: string; projectId: string } }) {
  const { track, last_activity } = summary;
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
        params={{ ...params, trackId: track.id }}
        className={rowClass}
      >
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium">{track.name}</span>
          {track.description && (
            <span className="mt-0.5 block truncate text-[13px] text-muted-foreground">{track.description.split(/\r?\n/, 1)[0]}</span>
          )}
        </span>
        <StateLabel state={track.state} />
        <span className="hidden w-28 shrink-0 text-right text-xs text-muted-foreground sm:inline">
          {last_activity ? timeAgo(last_activity) : "No activity yet"}
        </span>
      </Link>
    </li>
  );
}
