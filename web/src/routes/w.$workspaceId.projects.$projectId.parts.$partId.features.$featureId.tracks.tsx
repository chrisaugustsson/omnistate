import { Link, createFileRoute } from "@tanstack/react-router";
import { inFlight } from "@/components/shell/state-icon";
import { partLink } from "@/components/shell/scope";
import { useFeature } from "@/components/track/feature-data";
import { TracksByState } from "@/components/track/track-list";
import { EmptyLine, buttonClass } from "@/components/track/ui";
import { StartTrack } from "@/components/track/feature-parts";
import { useIsPhone } from "@/components/shell/shell-context";

// A feature's Tracks: the tracks that changed it or change it now, grouped by
// state; on the phone, Start track under them.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/tracks")({
  staticData: { section: "tracks" },
  component: Tracks,
});

function Tracks() {
  const data = useFeature();
  const { workspaceId, projectId, partId, tracks, people, part, feature } = data;
  const phone = useIsPhone();
  if (!tracks) return <div aria-hidden className="h-40 animate-pulse rounded-card bg-frame" />;
  const flying = tracks.filter((summary) => inFlight(summary.track.state)).length;
  return (
    <section aria-label="Tracks">
      <div className="mb-3 flex min-h-8 items-center gap-3">
        <p className="text-ink-3">
          <span className="font-medium text-ink">
            {tracks.length} {tracks.length === 1 ? "track" : "tracks"}
          </span>{" "}
          · {flying} in flight
        </p>
        <Link {...partLink(workspaceId, projectId, partId, "tracks")} className={buttonClass({ variant: "ghost" }, "-mr-2 ml-auto")}>
          All {part?.name ?? ""} tracks
        </Link>
      </div>
      {tracks.length === 0 ? (
        <EmptyLine>No track has changed {feature?.name ?? "this feature"} yet. Start a track to change it.</EmptyLine>
      ) : (
        <TracksByState tracks={tracks} params={{ workspaceId, projectId }} people={people} />
      )}
      {phone && <StartTrack data={data} className="mt-4 h-(--control-h-lg) w-full" />}
    </section>
  );
}
