import { Fragment } from "react";
import { Link } from "@tanstack/react-router";
import { Page } from "@/components/shell/page";
import { projectLink } from "@/components/shell/scope";
import { StateIcon, stateLabel } from "@/components/shell/state-icon";
import { STATE_ORDER, age, activityOf, myTracks, splitText } from "./model";
import { Band, ColumnList, EmptyRow, TrackRow } from "./rows";
import { findPart, useHomeData, useTrackFlags } from "./use-home-data";

// Home's My tracks (tasks/ui-redesign/v2/my-tracks.html): the tracks you own
// across every project and part, grouped by state as the Tracks list is, the
// most recently active first in each group. Planning, Building and Done always
// show, empty or not; Dropped only when a track was dropped.

/** What an empty group says. */
const EMPTY: Record<string, string> = {
  planning: "Nothing in planning.",
  building: "Nothing building.",
  done: "Nothing done yet.",
};

export function MyTracks({ workspaceId }: { workspaceId: string }) {
  const { projects, homes, parts, me, failed } = useHomeData(workspaceId);
  const mine = myTracks(homes, me);
  const flags = useTrackFlags(workspaceId, mine?.map(({ track }) => track) ?? null);
  const groups = STATE_ORDER.map((state) => ({ state, tracks: (mine ?? []).filter(({ track }) => track.state === state) })).filter(
    ({ state, tracks }) => state !== "dropped" || tracks.length > 0,
  );
  const dropped = mine?.some(({ track }) => track.state === "dropped");

  return (
    <Page>
      <div className="flex items-start gap-3">
        <div className="min-w-0">
          <h1 className="text-title font-semibold">My tracks</h1>
          <p className="mt-1.5 max-w-(--main-max) text-em text-ink-2">The tracks you own, across every part and project, by state.</p>
        </div>
      </div>
      {failed && <p className="mt-4 text-danger-ink">Your tracks could not be loaded. Try again in a moment.</p>}
      {mine && (
        <section aria-label="My tracks by state" className="mt-8">
          <ColumnList className="border-b-0">
            {groups.map(({ state, tracks }) => (
              <Fragment key={state}>
                <Band>
                  <StateIcon state={state} />
                  <h2>{stateLabel(state)}</h2>
                  <span className="font-medium text-ink-3 tabular-nums">{tracks.length}</span>
                </Band>
                {tracks.length === 0 ? (
                  <EmptyRow indent>{EMPTY[state]}</EmptyRow>
                ) : (
                  [...tracks]
                    .sort((a, b) => activityOf(b) - activityOf(a))
                    .map((summary) => (
                      <TrackRow
                        key={summary.track.id}
                        workspaceId={workspaceId}
                        summary={summary}
                        sub={splitText(summary, homes?.get(summary.track.project_id))}
                        flags={flags.get(summary.track.id)}
                        part={findPart(parts, summary.track.project_id, summary.track.part_id)}
                        parts={parts?.get(summary.track.project_id)}
                        end={
                          <span className="w-16 shrink-0 text-right text-meta whitespace-nowrap text-ink-3 tabular-nums">
                            {age(activityOf(summary))}
                          </span>
                        }
                      />
                    ))
                )}
              </Fragment>
            ))}
          </ColumnList>
          <p className="mt-2 border-t border-line pt-2 text-meta text-ink-3 [&_a]:font-medium [&_a]:text-ink-2 [&_a:hover]:text-ink [&_a:hover]:underline [&_a:hover]:decoration-line-strong [&_a:hover]:underline-offset-2">
            {dropped ? "" : "None dropped. "}Tracks other people own are on each project&rsquo;s{" "}
            {projects?.length === 1 ? (
              <Link {...projectLink(workspaceId, projects[0].id, "tracks")}>Tracks</Link>
            ) : (
              <>
                Tracks
                {projects && projects.length > 1 && (
                  <>
                    :{" "}
                    {projects.map((project, at) => (
                      <Fragment key={project.id}>
                        {at > 0 && ", "}
                        <Link {...projectLink(workspaceId, project.id, "tracks")}>{project.name}</Link>
                      </Fragment>
                    ))}
                  </>
                )}
              </>
            )}
            .
          </p>
        </section>
      )}
    </Page>
  );
}
