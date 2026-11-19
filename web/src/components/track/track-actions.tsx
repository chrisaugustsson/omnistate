import { useCallback, useState, type ReactNode } from "react";
import { ArrowRightLeft, Ellipsis, Link2, XCircle } from "lucide-react";
import { ApiError } from "@/api/client";
import { handOverTrack } from "@/api/members";
import { setTrackState } from "@/api/parts";
import { usePopover } from "@/components/shell/popover";
import { StateIcon } from "@/components/shell/state-icon";
import { Avatar } from "@/components/shell/avatar";
import { personName } from "@/lib/identity";
import { cn } from "@/lib/utils";
import type { MoveChoice, TrackData } from "./track-data";
import { Dialog } from "@/components/shell/dialog";
import { MenuHint, MenuSeparator, buttonClass, menuClass, menuItemClass, shortDate } from "./ui";

// What a reader does to a track from its header: move its state (the
// moves the API allows, only for its owner or an admin; others see them off
// with the reason), hand it over, and copy its link. A final move (done,
// dropped) asks first.

function moveError(failure: unknown, from: string, to: string): string {
  if (failure instanceof ApiError && failure.code === "invalid_transition") return `A track in ${from} cannot move to ${to}.`;
  if (failure instanceof ApiError && failure.code === "not_track_owner")
    return to === "done" ? "Only the owner can mark the track done." : "Only the owner or an admin can change the state.";
  return "The state could not be changed. Try again.";
}

function handOverError(failure: unknown): string {
  if (failure instanceof ApiError && failure.code === "not_track_owner") return "Only the owner or an admin can hand the track over.";
  if (failure instanceof ApiError && failure.code === "not_a_member") return "That user is not a member of this workspace.";
  return "The track could not be handed over. Try again.";
}

const STATE_OF: Record<string, string> = { building: "building", planning: "planning", done: "done", dropped: "dropped" };

export type TrackActions = {
  /** Makes a move, asking first when it is final. */
  request: (move: MoveChoice) => void;
  openHandOver: () => void;
  busy: boolean;
  error: string | null;
  /** The open dialog, if any; the layout renders it. */
  dialog: ReactNode;
};

/** The track's moves and hand-over, with their dialogs and the last error. */
export function useTrackActions(data: TrackData): TrackActions {
  const { workspaceId, projectId, trackId, summary } = data;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [asking, setAsking] = useState<MoveChoice | null>(null);
  const [handingOver, setHandingOver] = useState(false);
  const state = summary?.track.state ?? "";

  const move = useCallback(
    async (to: string) => {
      setBusy(true);
      setError(null);
      try {
        await setTrackState(workspaceId, projectId, trackId, to);
      } catch (failure) {
        setError(moveError(failure, state, to));
      } finally {
        setBusy(false);
      }
    },
    [workspaceId, projectId, trackId, state],
  );

  const request = useCallback(
    (choice: MoveChoice) => {
      if (!choice.allowed || busy) return;
      if (choice.final) setAsking(choice);
      else void move(choice.to);
    },
    [busy, move],
  );

  const close = useCallback(() => setAsking(null), []);
  const closeHandOver = useCallback(() => setHandingOver(false), []);
  const name = summary?.track.name ?? "this track";

  let dialog: ReactNode = null;
  if (asking) {
    dialog = (
      <Dialog
        title={asking.to === "done" ? `Mark ${name} done?` : `Drop ${name}?`}
        description={
          asking.to === "done"
            ? "Mark it done once its pull request has merged. A done track stays done."
            : "A dropped track stays dropped. Its docs, stories and decisions stay readable."
        }
        onClose={close}
        onSubmit={() => {
          close();
          void move(asking.to);
        }}
        footer={
          <>
            <button type="button" className={buttonClass()} onClick={close}>
              Cancel
            </button>
            <button type="submit" data-autofocus className={buttonClass({ variant: asking.danger ? "danger" : "primary" })}>
              {asking.label}
            </button>
          </>
        }
      />
    );
  } else if (handingOver) {
    dialog = <HandOverDialog data={data} onClose={closeHandOver} onError={setError} />;
  }

  return { request, openHandOver: () => setHandingOver(true), busy, error, dialog };
}

function HandOverDialog({ data, onClose, onError }: { data: TrackData; onClose: () => void; onError: (message: string | null) => void }) {
  const { workspaceId, projectId, trackId, summary, people } = data;
  const [choice, setChoice] = useState("");
  const [busy, setBusy] = useState(false);
  const others = (people?.members ?? []).filter((member) => member.user_id !== summary?.track.owner_user_id);

  async function handOver() {
    setBusy(true);
    onError(null);
    try {
      await handOverTrack(workspaceId, projectId, trackId, choice);
      onClose();
    } catch (failure) {
      onError(handOverError(failure));
      onClose();
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      title="Hand over"
      description="The new owner moves the track and records its decisions; agents run on their key."
      onClose={onClose}
      onSubmit={() => {
        if (choice && !busy) void handOver();
      }}
      footer={
        <>
          <button type="button" className={buttonClass()} onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className={buttonClass({ variant: "primary" })} disabled={!choice || busy}>
            Hand over
          </button>
        </>
      }
    >
      {others.length === 0 ? (
        <p className="text-ink-3">No other member of the workspace to hand the track over to.</p>
      ) : (
        <div role="radiogroup" aria-label="Hand over to" className="-mx-2 -my-1 max-h-64 overflow-y-auto">
          {others.map((member) => {
            const memberName = personName(member.email);
            return (
              <button
                key={member.membership_id}
                type="button"
                role="radio"
                aria-checked={choice === member.membership_id}
                onClick={() => setChoice(member.membership_id)}
                className="flex h-9 w-full items-center gap-2 rounded-control px-2 text-left text-ink hover:bg-hover aria-checked:bg-selected"
              >
                <Avatar name={memberName} tintKey={member.user_id} />
                <span className="font-medium">{memberName}</span>
                {member.role === "admin" && <span className="text-ink-3">admin</span>}
              </button>
            );
          })}
        </div>
      )}
    </Dialog>
  );
}

/** The header's actions: the "···" menu (hand over, copy link, the other moves, when the track was created) and the primary move, given as children. */
export function TrackHeaderActions({ data, actions, children }: { data: TrackData; actions: TrackActions; children?: ReactNode }) {
  const { moves, mayHandOver, ownerName, summary } = data;
  const primary = moves.find((one) => one.allowed && !one.danger) ?? moves.find((one) => !one.danger);
  const rest = moves.filter((one) => one !== primary);
  const menu = usePopover();
  const [copied, setCopied] = useState(false);
  const created = summary?.track.created_at;

  return (
    <div role="group" aria-label="Actions" className="flex shrink-0 items-center gap-2">
      <div ref={menu.anchor} className="relative inline-flex">
        <button
          ref={menu.opener}
          type="button"
          aria-label="More actions"
          aria-haspopup="menu"
          aria-expanded={menu.open}
          onClick={menu.toggle}
          className={buttonClass({ variant: "ghost", icon: true })}
        >
          <Ellipsis aria-hidden strokeWidth={2.25} />
        </button>
        {menu.open && (
          <div role="menu" aria-label="Track actions" className={menuClass}>
            <button
              type="button"
              role="menuitem"
              aria-disabled={!mayHandOver || undefined}
              title={mayHandOver ? undefined : `Only ${ownerName} or an admin hands it over`}
              onClick={() => {
                if (!mayHandOver) return;
                menu.setOpen(false);
                actions.openHandOver();
              }}
              className={menuItemClass}
            >
              <ArrowRightLeft aria-hidden strokeWidth={2.25} />
              Hand over…
              {!mayHandOver && <MenuHint>Owner or admin</MenuHint>}
            </button>
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                void navigator.clipboard?.writeText(window.location.href).then(() => setCopied(true), () => {});
                menu.setOpen(false);
              }}
              className={menuItemClass}
            >
              <Link2 aria-hidden strokeWidth={2.25} />
              {copied ? "Link copied" : "Copy link"}
            </button>
            {rest.length > 0 && <MenuSeparator />}
            {rest.map((one) => (
              <button
                key={one.to}
                type="button"
                role="menuitem"
                aria-disabled={!one.allowed || undefined}
                title={one.reason}
                onClick={() => {
                  if (!one.allowed) return;
                  menu.setOpen(false);
                  actions.request(one);
                }}
                className={cn(menuItemClass, one.danger && one.allowed && "text-danger-ink [&>svg]:text-danger")}
              >
                {one.danger ? <XCircle aria-hidden strokeWidth={2.25} /> : <StateIcon state={STATE_OF[one.to] ?? "backlog"} />}
                {one.danger ? `${one.label}…` : one.label}
                {!one.allowed && <MenuHint>{one.hint}</MenuHint>}
              </button>
            ))}
            {created && (
              <>
                <MenuSeparator />
                <p className="flex h-7 items-center px-2 text-meta text-ink-3" title={new Date(created).toLocaleString()}>
                  Created {shortDate(created)}
                </p>
              </>
            )}
          </div>
        )}
      </div>
      {children}
    </div>
  );
}
