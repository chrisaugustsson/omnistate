import { useCallback, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowRight, ArrowRightLeft, Ellipsis, Link2, PanelRight, XCircle } from "lucide-react";
import { ApiError } from "@/api/client";
import { handOverTrack } from "@/api/members";
import { setTrackState } from "@/api/parts";
import { usePopover } from "@/components/shell/popover";
import { StateIcon } from "@/components/shell/state-icon";
import { Avatar } from "@/components/shell/avatar";
import { personName } from "@/lib/identity";
import { cn } from "@/lib/utils";
import type { MoveChoice, TrackData } from "./track-data";
import { Dialog, MenuHint, MenuSeparator, buttonClass, menuClass, menuItemClass } from "./ui";

// What a reader does to a track from its header and panel: move its state (the
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
        onClose={close}
        actions={
          <>
            <button type="button" className={buttonClass()} onClick={close}>
              Cancel
            </button>
            <button
              type="button"
              className={buttonClass({ variant: asking.danger ? "danger" : "primary" })}
              onClick={() => {
                close();
                void move(asking.to);
              }}
            >
              {asking.label}
            </button>
          </>
        }
      >
        {asking.to === "done"
          ? "Mark it done once its pull request has merged. A done track stays done."
          : "A dropped track stays dropped. Its docs, stories and decisions stay readable."}
      </Dialog>
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
      onClose={onClose}
      actions={
        <>
          <button type="button" className={buttonClass()} onClick={onClose}>
            Cancel
          </button>
          <button type="button" className={buttonClass({ variant: "primary" })} disabled={!choice || busy} onClick={() => void handOver()}>
            Hand over
          </button>
        </>
      }
    >
      <p>The new owner moves the track and records its decisions; agents run on their key.</p>
      {others.length === 0 ? (
        <p className="mt-3 text-ink-3">No other member of the workspace to hand the track over to.</p>
      ) : (
        <div role="radiogroup" aria-label="Hand over to" className="mt-3 -mx-1 max-h-64 overflow-y-auto">
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

/** The header's actions: the properties toggle, the "…" menu and the primary move. */
export function TrackHeaderActions({
  data,
  actions,
  panel,
  onPanel,
}: {
  data: TrackData;
  actions: TrackActions;
  /** Whether the properties panel shows; left out, the page has no panel and no toggle (a canvas tab). */
  panel?: boolean;
  onPanel: () => void;
}) {
  const { moves, mayHandOver, ownerName } = data;
  const primary = moves.find((one) => one.allowed && !one.danger) ?? moves.find((one) => !one.danger);
  const rest = moves.filter((one) => one !== primary);
  const menu = usePopover();
  const [copied, setCopied] = useState(false);
  const [justToggled, setJustToggled] = useState(false);

  return (
    <div role="group" aria-label="Actions" className="flex items-center gap-2">
      {panel !== undefined && (
        <button
          type="button"
          aria-label="Properties"
          aria-pressed={panel}
          title={panel ? "Hide properties" : "Show properties"}
          onClick={() => {
            setJustToggled(true);
            onPanel();
          }}
          onPointerLeave={() => setJustToggled(false)}
          className={buttonClass(
            { variant: "ghost", icon: true },
            // The hover fill is all but the pressed one: right after a click that hides the panel it would
            // still look pressed under the pointer, so it waits until the pointer has left.
            justToggled && !panel ? "hover:bg-transparent hover:text-ink-2" : undefined,
          )}
        >
          <PanelRight aria-hidden strokeWidth={2.25} />
        </button>
      )}
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
          </div>
        )}
      </div>
      {primary && (
        <button
          type="button"
          className={buttonClass({ variant: "primary" })}
          disabled={!primary.allowed || actions.busy}
          title={primary.reason}
          onClick={() => actions.request(primary)}
        >
          {primary.label}
        </button>
      )}
    </div>
  );
}

/** The quiet row under the lede while behaviour questions wait: how many, and a way to them. */
export function NextRow({ data }: { data: TrackData }) {
  const { openQuestions, summary, workspaceId, projectId, trackId } = data;
  if (openQuestions === 0 || !summary) return null;
  const questions = `${openQuestions} behaviour ${openQuestions === 1 ? "question" : "questions"}`;
  return (
    <div className="mt-5 flex min-h-9 items-center gap-2.5 rounded-control bg-brand-tint py-1.5 pr-2 pl-3 text-ink">
      <ArrowRight aria-hidden strokeWidth={2.25} className="size-(--icon) shrink-0 text-brand" />
      <p className="min-w-0 flex-1">
        <b className="font-semibold">Next:</b> answer {questions}
        {summary.track.state === "planning" ? ", then start building." : "."}
      </p>
      <Link
        to="/w/$workspaceId/projects/$projectId/tracks/$trackId/behaviour"
        params={{ workspaceId, projectId, trackId }}
        className={buttonClass({ size: "sm" }, "border-transparent bg-sheet")}
      >
        Go to the questions
      </Link>
    </div>
  );
}
