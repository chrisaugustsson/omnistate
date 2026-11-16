import { useMemo, useState } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import { Search } from "lucide-react";
import type { TrackSummary } from "@/api/generated";
import { PageHeader } from "@/components/page-header";
import { matches } from "@/components/palette/search";
import { Empty, List } from "@/components/project/items";
import { NewTrack, StateLabel } from "@/components/project/new-track";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Input } from "@/components/ui/input";
import { timeAgo } from "@/lib/format";
import { useHome } from "@/lib/use-home";
import { useParts } from "@/lib/use-parts";

// The Tracks page: every track of the project in one flat list, the most
// recently worked on first, with a filter on name and description. Each row
// shows the track's part and state; a track on a deleted part shows the
// part's name marked as deleted. Creating a track asks for its part.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/tracks/")({
  component: Tracks,
});

/** A track's last activity, or its creation while it has none. */
const activityOf = ({ track, last_activity }: TrackSummary) => last_activity ?? track.created_at;

/** Most recent activity first, ties by name. */
function inOrder(tracks: TrackSummary[]): TrackSummary[] {
  return [...tracks].sort(
    (a, b) =>
      Date.parse(activityOf(b)) - Date.parse(activityOf(a)) ||
      a.track.name.toLowerCase().localeCompare(b.track.name.toLowerCase()),
  );
}

const firstLine = (text: string) => text.split(/\r?\n/, 1)[0];

function TrackRow({ summary, params }: { summary: TrackSummary; params: { workspaceId: string; projectId: string } }) {
  const { track, waiting, last_activity } = summary;
  const stories = summary.stories ?? { total: 0, open: 0, done: 0 };
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
        params={{ ...params, trackId: track.id }}
        className="group flex items-center gap-4 px-4 py-3 text-sm transition-colors hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none"
      >
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium">{track.name}</span>
          {track.description && <span className="mt-0.5 block truncate text-[13px] text-muted-foreground">{firstLine(track.description)}</span>}
        </span>
        <span className="hidden max-w-40 shrink-0 truncate text-xs text-muted-foreground sm:inline">
          {summary.part_name}
          {summary.part_deleted && " (deleted)"}
        </span>
        <StateLabel state={track.state} />
        <span className="hidden shrink-0 text-xs text-muted-foreground tabular-nums sm:inline">
          {stories.done}/{stories.total} stories done
        </span>
        <span className="flex w-8 shrink-0 justify-end">
          {waiting > 0 && (
            <span
              title="Waiting on you"
              className="rounded-full bg-signal px-1.5 text-center text-[11px] leading-5 text-signal-foreground tabular-nums"
            >
              {waiting}
            </span>
          )}
        </span>
        <span
          title={last_activity ? new Date(last_activity).toLocaleString() : undefined}
          className="hidden w-28 shrink-0 text-right text-xs text-muted-foreground sm:inline"
        >
          {last_activity ? timeAgo(last_activity) : "No activity yet"}
        </span>
      </Link>
    </li>
  );
}

function Tracks() {
  const { workspaceId, projectId } = Route.useParams();
  const { home, failed, reload } = useHome(workspaceId, projectId);
  const { parts } = useParts(workspaceId, projectId);
  const [query, setQuery] = useState("");
  const all = useMemo(() => inOrder(home?.tracks ?? []), [home]);
  const shown = useMemo(
    () => (query.trim() ? all.filter(({ track }) => matches(query, track.name, track.description)) : all),
    [all, query],
  );

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader title="Tracks" description="Every track of the project, the most recently worked on first." />
      {failed && (
        <Alert variant="destructive" className="mt-6">
          <AlertDescription>The page could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {home && (
        <div className="mt-6 space-y-4">
          <div className="flex items-center gap-3">
            <div className="relative max-w-md flex-1">
              <Search aria-hidden className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                type="text"
                aria-label="Filter tracks"
                placeholder="Filter by name or description"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                className="pl-9"
              />
            </div>
            <span className="ml-auto text-sm text-muted-foreground tabular-nums" aria-live="polite">
              {query.trim() ? `${shown.length} of ${all.length}` : `${all.length} ${all.length === 1 ? "track" : "tracks"}`}
            </span>
          </div>
          {parts && <NewTrack workspaceId={workspaceId} projectId={projectId} parts={parts} onCreated={reload} />}
          {all.length === 0 ? (
            <Empty>No tracks yet</Empty>
          ) : shown.length === 0 ? (
            <Empty>No track matches “{query.trim()}”. The filter looks at names and descriptions.</Empty>
          ) : (
            <List>
              {shown.map((summary) => (
                <TrackRow
                  key={summary.track.id}
                  summary={summary}
                  params={{ workspaceId, projectId }}
                />
              ))}
            </List>
          )}
        </div>
      )}
    </main>
  );
}
