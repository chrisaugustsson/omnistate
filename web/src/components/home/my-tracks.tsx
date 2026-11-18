import { Fragment } from "react";
import { Link } from "@tanstack/react-router";
import type { TrackSummary } from "@/api/generated";
import { Badge, Band, Progress, RowLead, ScopePage, SectionBar, rowClass } from "@/components/project/rows";
import { projectLink, trackLink } from "@/components/shell/scope";
import { StateIcon, stateLabel } from "@/components/shell/state-icon";
import { STATE_ORDER, age, activityOf, myTracks, splitText } from "./model";
import { findPart, useHomeData, useTrackFlags, type TrackFlags } from "./use-home-data";

// The workspace's My tracks: the tracks you own across every project and part,
// grouped by state as a Tracks list is, the most recently active first in each
// group. Planning, Building and Done always show, empty or not; Dropped only
// when a track was dropped. A row is the track's name over where it is and
// when it last moved, and one signal on the right: the behaviour questions
// waiting on you, else that it is blocked, else its progress.

/** What an empty group says. */
const EMPTY: Record<string, string> = {
  planning: "Nothing in planning.",
  building: "Nothing building.",
  done: "Nothing done yet.",
};

/** The one thing a row says on its right: questions waiting, else blocked, else progress. */
function Signal({ summary, flags }: { summary: TrackSummary; flags: TrackFlags | undefined }) {
  if (flags && flags.questions > 0) {
    return <Badge tone="brand">{flags.questions === 1 ? "1 question" : `${flags.questions} questions`}</Badge>;
  }
  if (flags?.blocked) {
    return (
      <span title={flags.blocked} className="inline-flex shrink-0 items-center gap-1.5 text-meta font-medium whitespace-nowrap text-danger-ink">
        <StateIcon state="blocked" />
        Blocked
        <span className="sr-only">: {flags.blocked}</span>
      </span>
    );
  }
  const stories = summary.stories;
  return stories && stories.total > 0 ? <Progress done={stories.done} total={stories.total} className="max-[480px]:[&>span]:hidden" /> : null;
}

export function MyTracks({ workspaceId }: { workspaceId: string }) {
  const { projects, homes, parts, me, failed } = useHomeData(workspaceId);
  const mine = myTracks(homes, me);
  const flags = useTrackFlags(workspaceId, mine?.map(({ track }) => track) ?? null);
  const groups = STATE_ORDER.map((state) => ({ state, tracks: (mine ?? []).filter(({ track }) => track.state === state) })).filter(
    ({ state, tracks }) => state !== "dropped" || tracks.length > 0,
  );
  const dropped = mine?.some(({ track }) => track.state === "dropped");
  const several = (projects?.length ?? 0) > 1;

  return (
    <ScopePage>
      <SectionBar title="My tracks" note="The tracks you own, across every part and project, by state." />
      {failed && <p className="mt-4 text-danger-ink">Your tracks could not be loaded. Try again in a moment.</p>}
      {mine && (
        <section aria-label="My tracks by state" className="mt-4">
          {groups.map(({ state, tracks }) => (
            <Fragment key={state}>
              <div className="mt-4 first:mt-0">
                <Band icon={<StateIcon state={state} />} count={tracks.length}>
                  <h2>{stateLabel(state)}</h2>
                </Band>
              </div>
              <ul aria-label={stateLabel(state)} className="flex flex-col">
                {tracks.length === 0 ? (
                  <li className="flex min-h-(--row-h) items-center py-2.5 pr-3 pl-12 text-ink-3">{EMPTY[state]}</li>
                ) : (
                  [...tracks]
                    .sort((a, b) => activityOf(b) - activityOf(a))
                    .map((summary) => {
                      const { track } = summary;
                      const part = findPart(parts, track.project_id, track.part_id)?.name ?? summary.part_name;
                      const project = several ? projects?.find((candidate) => candidate.id === track.project_id)?.name : undefined;
                      const where = [project, part, age(activityOf(summary)), splitText(summary, homes?.get(track.project_id))].filter(Boolean).join(" · ");
                      return (
                        <li key={track.id} className="[&:first-child>a]:border-t-0">
                          <Link {...trackLink(workspaceId, track.project_id, track.id, "spec")} className={rowClass({ two: true, link: true, inset: true })}>
                            <RowLead two>
                              <StateIcon state={track.state} label={stateLabel(track.state)} />
                            </RowLead>
                            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                              <span className="truncate font-medium">{track.name}</span>
                              <span className="truncate text-meta text-ink-3">{where}</span>
                            </span>
                            <span className="flex h-5 shrink-0 items-center">
                              <Signal summary={summary} flags={flags.get(track.id)} />
                            </span>
                          </Link>
                        </li>
                      );
                    })
                )}
              </ul>
            </Fragment>
          ))}
          <p className="mt-4 border-t border-line pt-3 text-meta text-ink-3 [&_a]:font-medium [&_a]:text-ink-2 [&_a:hover]:text-ink [&_a:hover]:underline [&_a:hover]:decoration-line-strong [&_a:hover]:underline-offset-2">
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
    </ScopePage>
  );
}
