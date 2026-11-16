import { useCallback, useState } from "react";
import { ApiError } from "@/api/client";
import { handOverTrack, loadMembers, loadMyUserId } from "@/api/members";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useReloading } from "@/lib/reloading";
import { useProjectEvents } from "@/live/context";

type Params = { workspaceId: string; projectId: string; trackId: string };

function messageOf(failure: unknown): string {
  if (failure instanceof ApiError && failure.code === "not_track_owner") return "Only the owner or an admin can hand the track over";
  if (failure instanceof ApiError && failure.code === "not_a_member") return "That user is not a member of this workspace";
  return "The track could not be handed over. Try again.";
}

/**
 * Hands a track to another member. Only the owner and admins see it; it lists the other
 * members by user ID, as `omni track handover` takes them.
 */
export function TrackOwner({ workspaceId, projectId, trackId, ownerUserId }: Params & { ownerUserId: string | null }) {
  const load = useCallback(async () => {
    const [members, me] = await Promise.all([loadMembers(workspaceId), loadMyUserId(workspaceId)]);
    return { members, me };
  }, [workspaceId]);
  const { value, reload } = useReloading(load);
  useProjectEvents(projectId, reload);
  const [choice, setChoice] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!value) return null;
  const mine = value.members.find((member) => member.user_id === value.me);
  const allowed = mine !== undefined && (mine.role === "admin" || mine.user_id === ownerUserId);
  if (!allowed) return null;
  const others = value.members.filter((member) => member.user_id !== ownerUserId);
  if (others.length === 0) return null;

  async function handOver() {
    setBusy(true);
    setError(null);
    try {
      await handOverTrack(workspaceId, projectId, trackId, choice);
      setChoice("");
    } catch (failure) {
      setError(messageOf(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section aria-labelledby="owner-title" className="space-y-3">
      <h2 id="owner-title" className="text-[15px] font-semibold tracking-tight">
        Owner
      </h2>
      <form
        className="flex gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (choice && !busy) void handOver();
        }}
      >
        <select
          aria-label="Hand over to"
          value={choice}
          className="h-8 max-w-xs rounded-lg border bg-background px-2 text-sm"
          onChange={(event) => {
            setChoice(event.target.value);
            setError(null);
          }}
        >
          <option value="">Hand over to...</option>
          {others.map((member) => (
            <option key={member.membership_id} value={member.membership_id}>
              {member.user_id}
            </option>
          ))}
        </select>
        <Button type="submit" size="sm" disabled={!choice || busy}>
          Hand over
        </Button>
      </form>
      {error && (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
    </section>
  );
}
