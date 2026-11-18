import { useState } from "react";
import { Link, Outlet, createFileRoute, useLocation } from "@tanstack/react-router";
import { ObjectHeader } from "@/components/shell/object-header";
import { Page } from "@/components/shell/page";
import { PathLink } from "@/components/shell/page-path";
import { partLink, projectLink, useScope } from "@/components/shell/scope";
import { countsKey, useReportSections } from "@/components/shell/shell-context";
import { NextRow, TrackHeaderActions, useTrackActions } from "@/components/track/track-actions";
import { TrackProvider, useTrackData } from "@/components/track/track-data";
import { TrackPanel } from "@/components/track/track-panel";
import { linkClass } from "@/components/track/ui";

// A track's pages. Its sections (Spec, Behaviour, Design, How, Board, Threads,
// Decisions) are the shell's sidebar, whose counts the layout reports: open
// behaviour questions and comments waiting on the reader are dark counts. The
// page starts with the path (Tracks › the track), the title with the track's
// actions (the properties toggle, the "…" menu and its next state move), its
// description and a quiet Next row while behaviour questions wait. The
// section's content sits beside the properties panel; the Board takes the full
// column until the panel is asked for. Behaviour and Design are canvases: there
// the header keeps its place without the description, the Next row and the
// properties, and the canvas fills the window under it. A doc opened full view
// fills the view instead (staticData.bleed). The layout loads the track's data
// once for every section.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId")({
  staticData: { level: "track" },
  component: TrackLayout,
});

const TRACK = "/w/$workspaceId/projects/$projectId/tracks/$trackId";

/** The tabs that want the full column: the panel stays closed there until it is asked for. */
const WIDE = new Set(["board"]);
/** The tabs that are a canvas: they fill the window under the header, with no panel. */
const CANVAS = new Set(["behaviour", "design"]);

function TrackLayout() {
  const { workspaceId, projectId, trackId } = Route.useParams();
  const { bleed } = useScope();
  const tab = useLocation({ select: (location) => location.pathname.split(`/tracks/${trackId}/`)[1]?.split("/")[0] ?? "" });
  const data = useTrackData(workspaceId, projectId, trackId, tab === "behaviour");
  const actions = useTrackActions(data);
  const wide = WIDE.has(tab);
  const canvas = CANVAS.has(tab);
  const [panelBeside, setPanelBeside] = useState(true);
  const [panelWide, setPanelWide] = useState(false);
  const panel = wide ? panelWide : panelBeside;
  const params = { workspaceId, projectId, trackId };
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
  const tracks = summary
    ? partLink(workspaceId, projectId, summary.track.part_id, "tracks")
    : projectLink(workspaceId, projectId, "tracks");

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
      <main className={canvas ? "flex h-full min-h-[560px] flex-col" : "flex min-h-full flex-col"}>
        <ObjectHeader
          wide={canvas}
          path={[
            <PathLink key="tracks" {...tracks} activeOptions={{ exact: true }}>
              Tracks
            </PathLink>,
            <PathLink key="track" to={TRACK} params={params} activeProps={{}} aria-current="page">
              {summary?.track.name ?? ""}
            </PathLink>,
          ]}
          title={summary?.track.name ?? ""}
          actions={
            summary && (
              <TrackHeaderActions
                data={data}
                actions={actions}
                panel={canvas ? undefined : panel}
                onPanel={() => (wide ? setPanelWide : setPanelBeside)(!panel)}
              />
            )
          }
          lede={!canvas && summary?.track.description ? <span className="whitespace-pre-line">{summary.track.description}</span> : undefined}
          notice={
            <>
              {!canvas && <NextRow data={data} />}
              {actions.error && (
                <p role="alert" className="mt-3 text-danger-ink">
                  {actions.error}
                </p>
              )}
            </>
          }
        />
        {canvas ? (
          <div className="flex min-h-0 flex-1 flex-col px-4 pt-3 pb-4">
            <Outlet />
          </div>
        ) : (
          <div className="mx-auto flex w-full max-w-(--page-max) flex-1 flex-col gap-12 px-4 pt-6 pb-10 md:px-(--page-pad) min-[960px]:flex-row">
            <div className="flex min-w-0 flex-1 flex-col">
              <Outlet />
            </div>
            {panel && summary && <TrackPanel data={data} actions={actions} className="w-full shrink-0 self-start min-[960px]:w-(--panel-w)" />}
          </div>
        )}
        {actions.dialog}
      </main>
    </TrackProvider>
  );
}
