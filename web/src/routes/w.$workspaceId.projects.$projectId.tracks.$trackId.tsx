import { Link, Outlet, createFileRoute } from "@tanstack/react-router";
import { Page } from "@/components/shell/page";
import { projectLink, useScope } from "@/components/shell/scope";
import { countsKey, useIsPhone, useReportSections } from "@/components/shell/shell-context";
import { useTrackActions } from "@/components/track/track-actions";
import { TrackProvider, useTrackData } from "@/components/track/track-data";
import { TrackHeader } from "@/components/track/track-header";
import { linkClass } from "@/components/track/ui";

// A track's pages. Its sections (Spec, Behaviour, Design, How, Board, Threads,
// Decisions) are the shell's sidebar, whose counts the layout reports: open
// behaviour questions and comments waiting on the reader are dark counts. The
// page starts with the track's header: the title with "···" and the primary
// state move, a line of facts and the description (components/track/track-header).
// A canvas section (Behaviour, Design, Board) fills the room beside the folded
// sidebar under a one-line header. On the phone the Board is a list and the
// Behaviour an outline, so they keep the full header and scroll as a page. A
// doc opened full view fills the view instead (staticData.bleed). The layout
// loads the track's data once for every section.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId")({
  staticData: { level: "track" },
  component: TrackLayout,
});

/** The sections that are a canvas on the phone too; Board and Behaviour become a list there. */
const PHONE_CANVAS = new Set(["design"]);

function TrackLayout() {
  const { workspaceId, projectId, trackId } = Route.useParams();
  const { bleed, canvas: wide, section } = useScope();
  const phone = useIsPhone();
  const data = useTrackData(workspaceId, projectId, trackId, section === "behaviour");
  const actions = useTrackActions(data);
  const canvas = wide && (!phone || PHONE_CANVAS.has(section ?? ""));
  const { summary, missing, docs, boards, onService, openQuestions, threads, decisions, home, isOwner } = data;
  const stories = summary?.stories;
  const showDesign = !onService || docs.design.length > 0;
  // The comments waiting on the reader: on the track's docs, when the track is theirs (the Inbox's rule).
  const waiting = isOwner ? (home?.waiting.comments ?? []).filter((comment) => comment.track_id === trackId).length : 0;
  useReportSections(countsKey("track", trackId), {
    behaviour: openQuestions > 0 ? { waiting: openQuestions, note: `${openQuestions} for you` } : {},
    design: !showDesign ? { hidden: true } : boards ? { count: boards, note: `${boards} ${boards === 1 ? "board" : "boards"}` } : {},
    board: stories ? { count: `${stories.done}/${stories.total}`, note: `${stories.done} of ${stories.total}` } : {},
    threads: waiting
      ? { waiting, note: `${waiting} for you` }
      : threads.length
        ? { count: threads.length, note: `${threads.length} ${threads.length === 1 ? "thread" : "threads"}` }
        : {},
    decisions: decisions?.length ? { count: decisions.length } : {},
  });
  if (bleed) {
    return (
      <TrackProvider value={data}>
        <Outlet />
      </TrackProvider>
    );
  }
  if (missing) {
    return (
      <Page>
        <h1 className="text-title font-semibold">Track not found</h1>
        <p className="mt-2 text-ink-2">
          The project has no such track, or it was deleted.{" "}
          <Link {...projectLink(workspaceId, projectId, "tracks")} className={linkClass}>
            Go to the project&rsquo;s tracks
          </Link>
        </p>
      </Page>
    );
  }
  return (
    <TrackProvider value={data}>
      <main className={canvas ? "flex h-full min-h-[480px] flex-col" : "flex min-h-full flex-col"}>
        <TrackHeader data={data} actions={actions} compact={canvas} lede={!wide} phone={phone} />
        {canvas ? (
          <div className="flex min-h-0 flex-1 flex-col">
            <Outlet />
          </div>
        ) : (
          <div className="mx-auto flex w-full max-w-(--page-max) min-w-0 flex-1 flex-col px-4 pt-6 pb-10 md:px-(--page-pad)">
            <Outlet />
          </div>
        )}
        {actions.dialog}
      </main>
    </TrackProvider>
  );
}
