import { Link } from "@tanstack/react-router";
import { Box, Lock } from "lucide-react";
import type { LinkedBody, TrackSummary } from "@/api/generated";
import { PartIcon } from "@/components/shell/part-icon";
import { PanelSection, PropertiesPanel, PropertyRow } from "@/components/shell/properties-panel";
import { StateIcon, StateLabel, stateLabel } from "@/components/shell/state-icon";
import { partTone } from "@/lib/identity";
import { cn } from "@/lib/utils";
import { nameOf, Person } from "./people";
import type { TrackActions } from "./track-actions";
import type { TrackData } from "./track-data";
import { Progress, chipClass, shortDate } from "./ui";

// A track's properties panel: its state and the next move, its owner and the
// hand-over, its part, the features it changes, its progress, the tracks it
// was split from and off to and the stories of other tracks that block its own,
// and when it was created.

const asideLink = "font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline disabled:no-underline";

const TRACK = "/w/$workspaceId/projects/$projectId/tracks/$trackId";
const BOARD = "/w/$workspaceId/projects/$projectId/tracks/$trackId/board";

function LinkRow({
  to,
  params,
  icon,
  title,
  meta,
  blocked = false,
}: {
  to: typeof TRACK | typeof BOARD;
  params: { workspaceId: string; projectId: string; trackId: string };
  icon: React.ReactNode;
  title: React.ReactNode;
  meta: React.ReactNode;
  blocked?: boolean;
}) {
  return (
    <Link
      to={to}
      params={params}
      className="grid grid-cols-[var(--icon-sm)_minmax(0,1fr)] gap-x-2 rounded-control px-1 py-1.5 transition-colors duration-(--t-fast) hover:bg-hover"
    >
      <span className="mt-[3px] grid">{icon}</span>
      <span className={cn("truncate font-medium", blocked ? "text-danger-ink" : "text-ink")}>{title}</span>
      <span className="col-start-2 flex flex-wrap items-center gap-1 text-meta text-ink-3">{meta}</span>
    </Link>
  );
}

function TrackLink({ data, summary, relation }: { data: TrackData; summary: TrackSummary; relation: string }) {
  const { workspaceId, projectId, parts, people } = data;
  const part = parts?.find((one) => one.id === summary.track.part_id);
  return (
    <LinkRow
      to={TRACK}
      params={{ workspaceId, projectId, trackId: summary.track.id }}
      icon={<StateIcon state={summary.track.state} />}
      title={summary.track.name}
      meta={
        <>
          {relation}
          {part && <PartIcon name={part.name} kind={part.kind} tone={partTone(parts, part.id)} size="sm" />}
          {summary.part_name ?? part?.name}
          {" · "}
          {nameOf(people, summary.track.owner_user_id)}
        </>
      }
    />
  );
}

function StoryKey({ children }: { children: React.ReactNode }) {
  return <span className="font-mono text-[12px]">{children}</span>;
}

/** A story of another track linked to ours: one that blocks ours, or one that ours blocks. */
function BlockerRow({ data, link }: { data: TrackData; link: LinkedBody }) {
  const { workspaceId, projectId } = data;
  const blocksUs = link.relation === "blocks";
  const params = { workspaceId, projectId, trackId: link.track_id };
  const board = BOARD;
  if (link.blocker_dropped) {
    return (
      <LinkRow
        to={board}
        params={params}
        icon={<StateIcon state="dropped" />}
        title={
          <>
            Blocker dropped: {link.part_name} <StoryKey>{link.key}</StoryKey>
          </>
        }
        meta={blocksUs ? <>Our <StoryKey>{link.story_key}</StoryKey> no longer waits on it</> : <>On {link.track_name}</>}
      />
    );
  }
  if (blocksUs) {
    return (
      <LinkRow
        to={board}
        params={params}
        blocked={link.status !== "done"}
        icon={<StateIcon state={link.status === "done" ? "done" : "blocked"} />}
        title={
          <>
            Blocked by {link.part_name} <StoryKey>{link.key}</StoryKey>
          </>
        }
        meta={
          <>
            Our <StoryKey>{link.story_key}</StoryKey> waits · theirs is {link.status === "done" ? "done" : `in ${stateLabel(link.status).toLowerCase()}`}
          </>
        }
      />
    );
  }
  return (
    <LinkRow
      to={board}
      params={params}
      icon={<StateIcon state={link.status} />}
      title={
        <>
          Blocks {link.part_name} <StoryKey>{link.key}</StoryKey>
        </>
      }
      meta={
        <>
          Their <StoryKey>{link.key}</StoryKey> waits on our <StoryKey>{link.story_key}</StoryKey>
        </>
      }
    />
  );
}

export function TrackPanel({ data, actions, className }: { data: TrackData; actions: TrackActions; className?: string }) {
  const { workspaceId, projectId, summary, part, tone, people, features, linked, tracks, moves, mayHandOver, ownerName } = data;
  if (!summary) return null;
  const { track } = summary;
  const next = moves.find((one) => one.allowed && !one.danger) ?? moves.find((one) => !one.danger);
  const readOnly = people !== null && !mayHandOver;
  const splitFrom = track.split_from_track_id ? tracks.find((one) => one.track.id === track.split_from_track_id) : undefined;
  const splitTo = tracks.filter((one) => one.track.split_from_track_id === track.id);
  const stories = summary.stories ?? { done: 0, total: 0, open: 0 };
  const hasLinks = !!splitFrom || splitTo.length > 0 || linked.length > 0;

  return (
    <PropertiesPanel className={className}>
      {readOnly && (
        <div className="mb-3 flex gap-2 rounded-control bg-frame px-2.5 py-2 text-meta text-ink-2">
          <Lock aria-hidden strokeWidth={2.25} className="mt-px size-(--icon-sm) shrink-0 text-ink-3" />
          <p>
            You can read and comment. {ownerName}, or an admin, changes this track.
          </p>
        </div>
      )}
      <PanelSection>
        <PropertyRow
          label="State"
          aside={
            next && (
              <>
                Next move:{" "}
                {next.allowed ? (
                  <button type="button" className={asideLink} disabled={actions.busy} onClick={() => actions.request(next)}>
                    {next.label}
                  </button>
                ) : (
                  next.label
                )}
              </>
            )
          }
        >
          <StateLabel state={track.state} className="h-6 px-1" />
        </PropertyRow>
        <PropertyRow
          label="Owner"
          aside={
            people &&
            (mayHandOver ? (
              <button type="button" className={asideLink} onClick={actions.openHandOver}>
                Hand over
              </button>
            ) : (
              `Only ${ownerName} or an admin hands it over`
            ))
          }
        >
          <Person people={people} userId={track.owner_user_id} className="h-6 px-1" />
        </PropertyRow>
        <PropertyRow label="Part">
          <Link to="/w/$workspaceId/projects/$projectId/parts/$partId" params={{ workspaceId, projectId, partId: track.part_id }} className={chipClass}>
            {part && <PartIcon name={part.name} kind={part.kind} tone={tone} size="sm" />}
            <span className="truncate">{summary.part_name ?? part?.name}</span>
          </Link>
          {part && <span className="text-meta text-ink-3">{part.kind}</span>}
        </PropertyRow>
        <PropertyRow label="Changes">
          {features && features.length === 0 && <span className="px-1 leading-6 text-ink-3">No feature</span>}
          {features?.map((feature) => (
            <Link
              key={feature.id}
              to="/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId"
              params={{ workspaceId, projectId, partId: feature.part_id, featureId: feature.id }}
              className={chipClass}
            >
              <Box aria-hidden strokeWidth={2.25} />
              <span className="truncate">{feature.name}</span>
            </Link>
          ))}
        </PropertyRow>
        <PropertyRow label="Progress">
          <Progress wide done={stories.done} total={stories.total} label={`${stories.done} of ${stories.total} ${stories.total === 1 ? "story" : "stories"}`} />
        </PropertyRow>
      </PanelSection>
      {hasLinks && (
        <PanelSection title="Linked work">
          {splitFrom && <TrackLink data={data} summary={splitFrom} relation="Split from" />}
          {splitTo.map((one) => (
            <TrackLink key={one.track.id} data={data} summary={one} relation="Split off to" />
          ))}
          {linked.map((link) => (
            <BlockerRow key={`${link.id}-${link.relation}-${link.story_id}`} data={data} link={link} />
          ))}
        </PanelSection>
      )}
      <PanelSection>
        <PropertyRow label="Created">
          <span className="px-1 leading-6" title={new Date(track.created_at).toLocaleString()}>
            {shortDate(track.created_at)}
          </span>
        </PropertyRow>
      </PanelSection>
    </PropertiesPanel>
  );
}
