import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { ApiError } from "@/api/client";
import type { TrackSummary } from "@/api/generated";
import { setTrackState } from "@/api/parts";
import { StateLabel } from "@/components/project/new-track";
import { memberLabel, useMembership } from "@/components/project/track-owner";
import { Button } from "@/components/ui/button";

// The line under a track's name: the part it is on, its state, its owner and
// the tracks it was split from and split off to (as `omni track show` prints
// them), and for the owner and admins the moves its state may make (US-311).

type Params = { workspaceId: string; projectId: string };

type Move = { to: string; label: string; ownerOnly?: boolean };

/** The moves out of each state; only the owner marks a track done. Done and dropped are final. */
const MOVES: Record<string, Move[]> = {
  planning: [
    { to: "building", label: "Start building" },
    { to: "dropped", label: "Drop" },
  ],
  building: [
    { to: "planning", label: "Back to planning" },
    { to: "done", label: "Mark done", ownerOnly: true },
    { to: "dropped", label: "Drop" },
  ],
};

function messageOf(failure: unknown, from: string, to: string): string {
  if (failure instanceof ApiError && failure.code === "invalid_transition") return `A track in ${from} cannot move to ${to}.`;
  if (failure instanceof ApiError && failure.code === "not_track_owner")
    return to === "done" ? "Only the owner can mark the track done." : "Only the owner or an admin can change the state.";
  return failure instanceof ApiError
    ? `The state could not be changed (${failure.code}). Try again.`
    : "The state could not be changed. Try again.";
}

const linkClass = "font-medium text-foreground underline-offset-4 hover:underline";

function TrackLink({ params, summary }: { params: Params; summary: TrackSummary }) {
  return (
    <Link to="/w/$workspaceId/projects/$projectId/tracks/$trackId" params={{ ...params, trackId: summary.track.id }} className={linkClass}>
      {summary.track.name}
    </Link>
  );
}

export function TrackStatus({ workspaceId, projectId, summary, tracks }: Params & { summary: TrackSummary; tracks: TrackSummary[] }) {
  const params = { workspaceId, projectId };
  const { track } = summary;
  const membership = useMembership(workspaceId, projectId);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const splitFrom = track.split_from_track_id ? tracks.find((one) => one.track.id === track.split_from_track_id) : undefined;
  const splitTo = tracks.filter((one) => one.track.split_from_track_id === track.id);
  const mine = membership?.members.find((member) => member.user_id === membership.me);
  const owner = mine !== undefined && mine.user_id === track.owner_user_id;
  const mayMove = owner || mine?.role === "admin";
  const moves = mayMove ? (MOVES[track.state] ?? []).filter((move) => owner || !move.ownerOnly) : [];

  async function move(to: string) {
    setBusy(true);
    setError(null);
    try {
      await setTrackState(workspaceId, projectId, track.id, to);
    } catch (failure) {
      setError(messageOf(failure, track.state, to));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-3 space-y-2">
      <div role="group" aria-label="Track details" className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-[13px]">
        <span>
          Part{" "}
          <Link to="/w/$workspaceId/projects/$projectId/parts/$partId" params={{ ...params, partId: track.part_id }} className={linkClass}>
            {summary.part_name ?? ""}
          </Link>
          {summary.part_deleted && " (deleted)"}
        </span>
        <StateLabel state={track.state} />
        {membership && (
          <span>
            Owner <span className="font-medium text-foreground">{memberLabel(track.owner_user_id, membership.me)}</span>
          </span>
        )}
        {splitFrom && (
          <span>
            Split from <TrackLink params={params} summary={splitFrom} />
          </span>
        )}
        {splitTo.length > 0 && (
          <span>
            Split off to{" "}
            {splitTo.map((one, index) => (
              <span key={one.track.id}>
                {index > 0 && ", "}
                <TrackLink params={params} summary={one} />
              </span>
            ))}
          </span>
        )}
      </div>
      {moves.length > 0 && (
        <div role="group" aria-label="Change state" className="flex flex-wrap gap-1.5">
          {moves.map((one) => (
            <Button key={one.to} type="button" variant="outline" size="sm" className="h-7 text-[12.5px]" disabled={busy} onClick={() => void move(one.to)}>
              {one.label}
            </Button>
          ))}
        </div>
      )}
      {error && (
        <p role="alert" className="text-[13px] text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}
