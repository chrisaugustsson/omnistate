import { Fragment, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import type { LinkedBody, TrackSummary } from "@/api/generated";
import { StateIcon, stateLabel } from "@/components/shell/state-icon";
import { cn } from "@/lib/utils";
import { TrackHeaderActions, type TrackActions } from "./track-actions";
import type { TrackData } from "./track-data";
import { buttonClass } from "./ui";

// A track's header (navigation F, tasks/ui-redesign/build-f.md): the title with
// "···" and the primary state move on the right, one line of facts under it
// (its state, the features it changes, its owner, the tracks it was split from
// and off to, and the tracks whose stories block its own), and its description.
// The path header above already names the part, so the facts leave it out. On
// a canvas section (Behaviour, Design, Board) the header is one line: the
// title, the facts as far as they fit, and the actions. On the phone the
// primary move takes the full width under the facts.

const factLink =
  "font-medium text-ink underline decoration-line-strong underline-offset-[3px] transition-colors duration-(--t-fast) hover:decoration-ink-3 focus-visible:shadow-focus focus-visible:outline-none";

const TRACK = "/w/$workspaceId/projects/$projectId/tracks/$trackId";
const BOARD = "/w/$workspaceId/projects/$projectId/tracks/$trackId/board";
const FEATURE = "/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId";

/** Names joined in running text: "A", "A and B", "A, B and C". */
function joined(items: ReactNode[]): ReactNode {
  return items.map((item, at) => (
    <Fragment key={at}>
      {at > 0 && (at === items.length - 1 ? " and " : ", ")}
      {item}
    </Fragment>
  ));
}

/** The other tracks whose open stories block this track's own, each with the keys involved. */
export function blockingTracks(linked: LinkedBody[]): { trackId: string; name: string; keys: string[] }[] {
  const byTrack = new Map<string, { trackId: string; name: string; keys: string[] }>();
  for (const link of linked) {
    if (link.relation !== "blocks" || link.status === "done" || link.status === "cancelled") continue;
    const entry = byTrack.get(link.track_id) ?? { trackId: link.track_id, name: link.track_name, keys: [] };
    entry.keys.push(`${link.part_name} ${link.key} blocks our ${link.story_key}`);
    byTrack.set(link.track_id, entry);
  }
  return [...byTrack.values()];
}

/** The facts line's items, each a link where it has a target. */
function Facts({ data, actions, className }: { data: TrackData; actions: TrackActions; className?: string }) {
  const { workspaceId, projectId, summary, features, tracks, linked, people, mayHandOver, ownerName } = data;
  if (!summary) return null;
  const { track } = summary;
  const splitFrom = track.split_from_track_id ? tracks.find((one) => one.track.id === track.split_from_track_id) : undefined;
  const splitTo = tracks.filter((one) => one.track.split_from_track_id === track.id);
  const blockers = blockingTracks(linked);
  const trackLink = (one: TrackSummary) => (
    <Link key={one.track.id} to={TRACK} params={{ workspaceId, projectId, trackId: one.track.id }} className={factLink}>
      {one.track.name}
    </Link>
  );

  const facts: { key: string; node: ReactNode }[] = [
    {
      key: "state",
      node: (
        <span className="font-medium text-ink">
          <StateIcon state={track.state} className="mr-1.5 inline-block align-[-2px]" />
          {stateLabel(track.state)}
        </span>
      ),
    },
  ];
  if (features && features.length > 0) {
    facts.push({
      key: "changes",
      node: (
        <>
          changes{" "}
          {joined(
            features.map((feature) => (
              <Link
                key={feature.id}
                to={FEATURE}
                params={{ workspaceId, projectId, partId: feature.part_id, featureId: feature.id }}
                className={factLink}
              >
                {feature.name}
              </Link>
            )),
          )}
        </>
      ),
    });
  }
  if (people) {
    facts.push({
      key: "owner",
      node: (
        <>
          owner{" "}
          {mayHandOver ? (
            <button
              type="button"
              aria-haspopup="dialog"
              aria-label={`Owner ${ownerName}: hand over`}
              title="Hand over"
              onClick={actions.openHandOver}
              className={factLink}
            >
              {ownerName}
            </button>
          ) : (
            <span title={`Only ${ownerName} or an admin hands it over`} className="font-medium text-ink">
              {ownerName}
            </span>
          )}
        </>
      ),
    });
  }
  if (splitFrom) facts.push({ key: "from", node: <>split off from {trackLink(splitFrom)}</> });
  if (splitTo.length > 0) facts.push({ key: "to", node: <>split off to {joined(splitTo.map(trackLink))}</> });
  if (blockers.length > 0) {
    facts.push({
      key: "blocked",
      node: (
        <>
          <span className="text-danger-ink">blocked by</span>{" "}
          {joined(
            blockers.map((blocker) => (
              <Link
                key={blocker.trackId}
                to={BOARD}
                params={{ workspaceId, projectId, trackId: blocker.trackId }}
                title={blocker.keys.join("; ")}
                className={factLink}
              >
                {blocker.name}
              </Link>
            )),
          )}
        </>
      ),
    });
  }

  return (
    <p aria-label="Facts" className={cn("text-ink-2", className)}>
      {facts.map((fact, at) => (
        <span key={fact.key} className="max-w-full">
          {fact.node}
          {at < facts.length - 1 && (
            <span aria-hidden className="mx-[7px] text-ink-3">
              ·
            </span>
          )}
        </span>
      ))}
    </p>
  );
}

/** The primary state move: the first move the reader may make, else the first that is not a danger. */
function primaryOf(data: TrackData) {
  return data.moves.find((one) => one.allowed && !one.danger) ?? data.moves.find((one) => !one.danger);
}

function PrimaryMove({ data, actions, className }: { data: TrackData; actions: TrackActions; className?: string }) {
  const primary = primaryOf(data);
  if (!primary) return null;
  return (
    <button
      type="button"
      className={buttonClass({ variant: "primary" }, className)}
      disabled={!primary.allowed || actions.busy}
      title={primary.reason}
      onClick={() => actions.request(primary)}
    >
      {primary.label}
    </button>
  );
}

export function TrackHeader({
  data,
  actions,
  compact = false,
  lede = true,
  phone = false,
}: {
  data: TrackData;
  actions: TrackActions;
  /** Over a canvas section: one line, the width of the window. */
  compact?: boolean;
  /** Shows the description under the facts. */
  lede?: boolean;
  phone?: boolean;
}) {
  const { summary } = data;
  const name = summary?.track.name ?? "";
  const error = actions.error && (
    <p role="alert" className={cn("text-danger-ink", compact ? "truncate text-meta" : "mt-3")}>
      {actions.error}
    </p>
  );

  if (compact) {
    return (
      <div className="flex h-12 w-full shrink-0 items-center gap-3 border-b border-line px-4">
        {/* On the phone the header above names the track; the line keeps its facts. */}
        <h1 className={phone ? "sr-only" : "max-w-[45%] min-w-0 shrink-0 truncate font-semibold text-ink"} title={name}>
          {name}
        </h1>
        {summary && <Facts data={data} actions={actions} className="min-w-0 flex-1 truncate text-meta whitespace-nowrap" />}
        {error}
        {summary && (
          <TrackHeaderActions data={data} actions={actions}>
            <PrimaryMove data={data} actions={actions} className={phone ? "px-2" : undefined} />
          </TrackHeaderActions>
        )}
      </div>
    );
  }

  return (
    <div className="mx-auto w-full max-w-(--page-max) px-4 pt-5 md:px-(--page-pad) md:pt-7">
      <div className="flex items-start gap-3">
        <h1 className="min-w-0 flex-1 pt-px text-title font-semibold text-pretty">{name}</h1>
        {summary && (
          <TrackHeaderActions data={data} actions={actions}>
            {!phone && <PrimaryMove data={data} actions={actions} />}
          </TrackHeaderActions>
        )}
      </div>
      {summary && <Facts data={data} actions={actions} className="mt-1.5 flex flex-wrap gap-y-1 md:[&>span]:whitespace-nowrap" />}
      {phone && summary && primaryOf(data) && <PrimaryMove data={data} actions={actions} className="mt-4 h-(--control-h-lg) w-full" />}
      {lede && summary?.track.description && <p className="mt-3 max-w-(--main-max) text-em whitespace-pre-line text-ink-2">{summary.track.description}</p>}
      {error}
    </div>
  );
}
