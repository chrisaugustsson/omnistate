import { Outlet, createFileRoute } from "@tanstack/react-router";
import { ObjectHeader } from "@/components/shell/object-header";
import { TabLink } from "@/components/shell/in-page-tabs";
import { PathLink } from "@/components/shell/page-path";
import { partLink, projectLink, useScope } from "@/components/shell/scope";
import { useHome } from "@/lib/use-home";

// A track's pages. The bar keeps the track's part with Tracks lit; the page
// starts with the path (Tracks › the track) and the track's own tabs: Spec,
// Behaviour, Design, How, Board, Threads, Decisions. Slice S3 builds the tabs'
// content, the title row and the properties panel; until then each tab's old
// page brings its own title. The Behaviour screen and an open doc fill the view
// instead (staticData.bleed).
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId")({
  staticData: { scope: "track", tab: "tracks" },
  component: TrackLayout,
});

const TRACK = "/w/$workspaceId/projects/$projectId/tracks/$trackId";

function TrackLayout() {
  const { workspaceId, projectId, trackId } = Route.useParams();
  const { bleed } = useScope();
  const { home } = useHome(workspaceId, projectId);
  const summary = home?.tracks?.find((candidate) => candidate.track.id === trackId);
  const params = { workspaceId, projectId, trackId };
  // No header while the page fills the view, nor for a track the project does not have.
  const header = !bleed && !(home && !summary);
  const stories = summary?.stories;
  const threads = home?.threads.filter((thread) => thread.track_id === trackId).length;
  const tracks = summary
    ? partLink(workspaceId, projectId, summary.track.part_id, "tracks")
    : projectLink(workspaceId, projectId, "tracks");
  return (
    <>
      {header && (
        <ObjectHeader
          path={[
            <PathLink key="tracks" {...tracks} activeOptions={{ exact: true }}>
              Tracks
            </PathLink>,
            <PathLink key="track" to={TRACK} params={params} activeProps={{}} aria-current="page">
              {summary?.track.name ?? ""}
            </PathLink>,
          ]}
          tabsLabel="Track"
          tabs={
            <>
              <TabLink to={TRACK} params={params} activeOptions={{ exact: true }}>
                Spec
              </TabLink>
              <TabLink to={`${TRACK}/behaviour`} params={params}>
                Behaviour
              </TabLink>
              <TabLink to={`${TRACK}/design`} params={params}>
                Design
              </TabLink>
              <TabLink to={`${TRACK}/how`} params={params}>
                How
              </TabLink>
              <TabLink to={`${TRACK}/board`} params={params} count={stories ? `${stories.done}/${stories.total}` : undefined}>
                Board
              </TabLink>
              <TabLink to={`${TRACK}/threads`} params={params} count={threads || undefined}>
                Threads
              </TabLink>
              <TabLink to={`${TRACK}/decisions`} params={params}>
                Decisions
              </TabLink>
            </>
          }
        />
      )}
      {/* The outlet keeps its place whether the header shows or not, so the tab's page is not mounted again. */}
      <Outlet />
    </>
  );
}
