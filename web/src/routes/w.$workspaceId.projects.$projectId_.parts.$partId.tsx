import { Link, createFileRoute } from "@tanstack/react-router";
import type { TrackSummary } from "@/api/generated";
import { PageHeader } from "@/components/page-header";
import { NewTrack, StateLabel } from "@/components/project/new-track";
import { KindLabel } from "@/components/project/parts-section";
import { Empty, List, rowClass } from "@/components/project/items";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { timeAgo } from "@/lib/format";
import { useHome } from "@/lib/use-home";
import { useParts } from "@/lib/use-parts";

// A part's page: what the part is (its description), its features (the
// capabilities that US-307 adds) and its tracks, the active ones first, then
// done and dropped, each group with the most recently updated first.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/parts/$partId")({
  component: PartPage,
});

const FINISHED = new Set(["done", "dropped"]);

/** A track's last activity, or its creation while it has none. */
const updatedAt = ({ track, last_activity }: TrackSummary) => Date.parse(last_activity ?? track.created_at);

const byRecent = (a: TrackSummary, b: TrackSummary) =>
  updatedAt(b) - updatedAt(a) || a.track.name.toLowerCase().localeCompare(b.track.name.toLowerCase());

function TrackRow({ summary, params }: { summary: TrackSummary; params: { workspaceId: string; projectId: string } }) {
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

function PartPage() {
  const { workspaceId, projectId, partId } = Route.useParams();
  const params = { workspaceId, projectId };
  const { parts, failed: partsFailed } = useParts(workspaceId, projectId);
  const { home, failed: homeFailed, reload } = useHome(workspaceId, projectId);
  const part = parts?.find((candidate) => candidate.id === partId);
  const missing = parts !== null && !part;
  const tracks = (home?.tracks ?? []).filter((summary) => summary.track.part_id === partId);
  const ordered = [
    ...tracks.filter((summary) => !FINISHED.has(summary.track.state)).sort(byRecent),
    ...tracks.filter((summary) => FINISHED.has(summary.track.state)).sort(byRecent),
  ];

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      {(partsFailed || homeFailed) && (
        <Alert variant="destructive">
          <AlertDescription>The page could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {missing && (
        <div className="space-y-3">
          <h1 className="font-display text-[2.125rem] leading-[1.1] tracking-[-0.01em]">Part not found</h1>
          <p className="text-[15px] text-muted-foreground">
            The project has no such part, or it was deleted.{" "}
            <Link
              to="/w/$workspaceId/projects/$projectId/app"
              params={params}
              hash="parts"
              className="font-medium text-foreground underline underline-offset-4"
            >
              Go to the project
            </Link>
          </p>
        </div>
      )}
      {part && (
        <>
          <PageHeader
            eyebrow={
              <Link
                to="/w/$workspaceId/projects/$projectId/app"
                params={params}
                hash="parts"
                className="hover:text-foreground"
              >
                Parts
              </Link>
            }
            title={part.name}
            badge={<KindLabel kind={part.kind} />}
            description={part.description ? <p className="whitespace-pre-line">{part.description}</p> : undefined}
          />
          <div className="mt-10 space-y-12">
            <section aria-labelledby="features-title" className="space-y-3">
              <div className="flex items-baseline gap-2.5">
                <h2 id="features-title" className="text-[15px] font-semibold tracking-tight">
                  Features
                </h2>
                <span className="text-sm text-muted-foreground tabular-nums">0</span>
              </div>
              <Empty>No features yet</Empty>
            </section>
            <section aria-labelledby="tracks-title" className="space-y-3">
              <div className="flex flex-wrap items-center gap-2.5">
                <h2 id="tracks-title" className="text-[15px] font-semibold tracking-tight">
                  Tracks
                </h2>
                <span className="text-sm text-muted-foreground tabular-nums">{ordered.length}</span>
              </div>
              <NewTrack {...params} fixedPartId={part.id} onCreated={reload} />
              {home &&
                (ordered.length === 0 ? (
                  <Empty>No tracks on this part yet</Empty>
                ) : (
                  <List>
                    {ordered.map((summary) => (
                      <TrackRow key={summary.track.id} summary={summary} params={params} />
                    ))}
                  </List>
                ))}
            </section>
          </div>
        </>
      )}
    </main>
  );
}
