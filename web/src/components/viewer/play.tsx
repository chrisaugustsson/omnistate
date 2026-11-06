import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { FloatingAlerts } from "@/components/viewer/chrome";
import type { DesignRenderer, PlayState } from "@/design/renderer";

// Play mode: a prototype played full window over the design viewer. The
// renderer draws the board the player shows on a canvas that lives in the stage
// for as long as the session lasts, and takes the pointer; this component is the
// two bars (Exit and the flow on top, Restart, Previous, Next and Back below)
// and keeps the address bar on the board the player is on. Transitions are
// instant, so a click on a hotspot shows its board at once. A link asks first
// (LinkNotice), a click on nothing says so in a toast while the renderer
// outlines the hotspots, and Cmd/Ctrl+\ hides the bars.

export type PlayBoard = { id: string; name: string };
export type PlayFlow = { name: string; start: string };

const bar = "flex h-11 shrink-0 items-center gap-3 bg-neutral-800 px-3 text-[13px] text-neutral-100";
const control =
  "h-7 rounded-md px-2.5 text-[12px] text-neutral-100 outline-none transition-colors hover:bg-neutral-700 focus-visible:ring-[3px] focus-visible:ring-white/30 disabled:pointer-events-none disabled:text-neutral-500";

const noticeKey = "omni.play.linkNotice";
const toastMs = 4000;
const mac = () => /Mac|iPhone|iPad/.test(navigator.platform);

function skipsNotice() {
  try {
    return localStorage.getItem(noticeKey) === "1";
  } catch {
    return false;
  }
}

function openLink({ url, newTab }: Link) {
  if (newTab) window.open(url, "_blank", "noopener");
  else window.location.assign(url);
}

type Link = { url: string; newTab: boolean };

/** What the notice says a link does. */
function leaves({ url }: Link) {
  if (!url.toLowerCase().startsWith("mailto:")) return <>This opens {url} outside the prototype.</>;
  const address = url.slice("mailto:".length).split("?")[0];
  let shown = address;
  try {
    shown = decodeURIComponent(address);
  } catch {
    // An address that is not valid percent-encoding is shown as written.
  }
  return <>This opens your mail app to write to {shown}.</>;
}

/** Asks before a link takes the reader out of the prototype. */
function LinkNotice({ link, onClose }: { link: Link; onClose: () => void }) {
  const [skip, setSkip] = useState(false);
  return (
    <div className="absolute inset-0 z-30 flex items-center justify-center bg-black/50 p-6">
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="play-link-title"
        aria-describedby="play-link-body"
        className="flex w-full max-w-sm flex-col gap-3 rounded-xl bg-neutral-800 p-5 text-neutral-100 shadow-panel"
      >
        <h2 id="play-link-title" className="text-[15px] font-semibold">
          Open link?
        </h2>
        <p id="play-link-body" className="text-[13px] leading-snug break-words text-neutral-300">
          {leaves(link)}
        </p>
        <label className="flex items-center gap-2 text-[12px] text-neutral-300">
          <input type="checkbox" checked={skip} onChange={(event) => setSkip(event.target.checked)} />
          Don't show this again
        </label>
        <div className="flex justify-end gap-2">
          <button type="button" className={control} onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            autoFocus
            className={`${control} bg-neutral-100 font-medium text-neutral-900 hover:bg-white`}
            onClick={() => {
              if (skip) {
                try {
                  localStorage.setItem(noticeKey, "1");
                } catch {
                  // Without storage the notice shows again, which is safe.
                }
              }
              onClose();
              openLink(link);
            }}
          >
            Open link
          </button>
        </div>
      </div>
    </div>
  );
}

function reason(error: unknown) {
  return error instanceof Error && error.message ? error.message : "The prototype could not be played.";
}

/**
 * Plays `play`, a board of the page the renderer shows, in a dialog over the
 * window. `ready` resolves once the renderer has every board of the page read.
 * `onPlay` is told the board the player is on (and the flow), so that the
 * address follows it; `onExit` is told the board to return to in the design.
 */
export function PlayMode({
  renderer,
  title,
  boards,
  flows,
  play,
  flow,
  ready,
  onPlay,
  onExit,
}: {
  renderer: DesignRenderer;
  title: string;
  /** The boards of the page, in the page's order. */
  boards: PlayBoard[];
  flows: PlayFlow[];
  /** The board the address names. */
  play: string;
  /** The flow the address names. */
  flow: string | undefined;
  ready: () => Promise<void>;
  onPlay: (board: string, flow: string | undefined) => void;
  onExit: (board: string) => void;
}) {
  const stage = useRef<HTMLDivElement>(null);
  const [state, setState] = useState<PlayState | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The board the session started on, where Restart goes when no flow is chosen.
  const started = useRef(play);
  // The board the address was last told of or follows, and the boards it was told of that it has not shown yet.
  const last = useRef(play);
  // The address as the last render saw it, to tell it moving from the player moving.
  const seen = useRef(play);
  const pushed = useRef<string[]>([]);
  const [notice, setNotice] = useState<Link | null>(null);
  const [hidden, setHidden] = useState(false);
  const [toast, setToast] = useState<{ text: string; id: number } | null>(null);
  const latest = useRef({ ready, onExit, state, play, notice, hidden });
  useEffect(() => {
    latest.current = { ready, onExit, state, play, notice, hidden };
  });
  const toasts = useRef(0);
  function say(text: string) {
    toasts.current += 1;
    setToast({ text, id: toasts.current });
  }
  // A toast goes by itself; a newer one restarts the time.
  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), toastMs);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    let cancelled = false;
    let running = false;
    const stop = renderer.play.onChange(() => setState(renderer.play.state()));
    latest.current.ready().then(
      () => {
        const element = stage.current;
        if (cancelled || !element) return;
        try {
          renderer.play.start(latest.current.play, element);
          running = true;
          setState(renderer.play.state());
        } catch (failure) {
          setError(reason(failure));
        }
      },
      (failure: unknown) => {
        if (!cancelled) setError(reason(failure));
      },
    );
    return () => {
      cancelled = true;
      stop();
      if (running) renderer.play.stop();
    };
  }, [renderer]);

  // The address follows the board the player is on; an address that moves on its own starts the session over there.
  useEffect(() => {
    if (!state) return;
    const moved = play !== seen.current;
    seen.current = play;
    if (moved) {
      const at = pushed.current.indexOf(play);
      if (at >= 0) {
        pushed.current.splice(0, at + 1);
      } else if (play !== state.board) {
        last.current = play;
        try {
          renderer.play.restart(play);
        } catch (failure) {
          setError(reason(failure));
        }
        return;
      }
    }
    if (state.board === last.current) return;
    last.current = state.board;
    if (state.board !== play) {
      pushed.current.push(state.board);
      onPlay(state.board, flow);
    }
  }, [renderer, state, play, flow, onPlay]);

  // A link the prototype asked to open; one that comes while the notice shows is dropped.
  useEffect(() => {
    const link = state?.url;
    if (!link) return;
    if (skipsNotice()) openLink(link);
    else setNotice((showing) => showing ?? link);
  }, [state]);

  // A click on nothing: the renderer outlines the hotspots, this says so.
  useEffect(() => {
    if (state?.missed) say("Nothing happens here. The hotspots are outlined.");
  }, [state]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "\\" && (mac() ? event.metaKey : event.ctrlKey) && !event.altKey) {
        event.preventDefault();
        // The prototype's own key triggers do not see it.
        event.stopPropagation();
        const hide = !latest.current.hidden;
        setHidden(hide);
        if (hide) say(`Press ${mac() ? "⌘" : "Ctrl+"}\\ to show the bars again`);
        return;
      }
      if (event.key !== "Escape") return;
      event.preventDefault();
      // The design view's own Escape handling would clear its selection.
      event.stopPropagation();
      // Escape answers the notice as Cancel before it leaves play mode.
      if (latest.current.notice) {
        setNotice(null);
        return;
      }
      const { onExit: exit, state: now, play: named } = latest.current;
      exit(now?.board ?? named);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);

  const chosen = flows.find((candidate) => candidate.name === flow);
  const at = state ? boards.findIndex((candidate) => candidate.id === state.board) : -1;

  function act(change: () => void) {
    try {
      change();
    } catch (failure) {
      setError(reason(failure));
    }
  }

  function restartAt(board: string, name: string | undefined) {
    renderer.play.restart(board);
    if (board === last.current && name === flow) return;
    last.current = board;
    if (board !== play) pushed.current.push(board);
    onPlay(board, name);
  }

  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Play mode"
      className="fixed inset-0 z-50 flex flex-col bg-neutral-900 text-neutral-100"
    >
      <div className={`${bar} ${hidden ? "invisible" : ""}`} aria-hidden={hidden}>
        <button
          type="button"
          className={control}
          onClick={() => onExit(state?.board ?? play)}
        >
          ← Exit
        </button>
        <span className="min-w-0 flex-1 truncate text-neutral-400">{title}</span>
        {flows.length > 0 && (
          <label className="flex items-center gap-1.5 text-[12px] text-neutral-400">
            Flow
            <select
              className="h-7 max-w-56 rounded-md border-0 bg-neutral-700 px-2 text-[12px] font-medium text-neutral-100 outline-none focus-visible:ring-[3px] focus-visible:ring-white/30"
              value={chosen?.name ?? ""}
              onChange={(event) => {
                const next = flows.find((candidate) => candidate.name === event.target.value);
                if (next) act(() => restartAt(next.start, next.name));
              }}
            >
              {!chosen && <option value="" disabled />}
              {flows.map((candidate) => (
                <option key={candidate.name} value={candidate.name}>
                  {candidate.name}
                </option>
              ))}
            </select>
          </label>
        )}
      </div>
      <div ref={stage} className="relative min-h-0 flex-1 overflow-hidden bg-[#1e1e1e]">
        {error ? (
          <p role="alert" className="absolute inset-0 flex items-center justify-center p-6 text-center text-[13px] text-neutral-300">
            {error}
          </p>
        ) : (
          !state && (
            <p className="absolute inset-0 flex items-center justify-center text-[13px] text-neutral-400">Loading…</p>
          )
        )}
        {toast && (
          <FloatingAlerts>
            <p
              key={toast.id}
              role="status"
              className="pointer-events-auto rounded-lg bg-neutral-700 px-3.5 py-2 text-[12px] text-neutral-100 shadow-panel"
            >
              {toast.text}
            </p>
          </FloatingAlerts>
        )}
        {notice && <LinkNotice link={notice} onClose={() => setNotice(null)} />}
      </div>
      <div className={`${bar} justify-center gap-4 ${hidden ? "invisible" : ""}`} aria-hidden={hidden}>
        <button
          type="button"
          className={control}
          disabled={!state}
          onClick={() => act(() => restartAt(chosen?.start ?? started.current, chosen?.name))}
        >
          ↺ Restart
        </button>
        <button
          type="button"
          className={control}
          disabled={!state || at <= 0}
          onClick={() => act(() => renderer.play.go(boards[at - 1].id))}
        >
          ‹ Previous
        </button>
        <span className="min-w-32 text-center text-[12px] font-medium">
          {at >= 0 ? `${boards[at].name} · ${at + 1} of ${boards.length}` : ""}
        </span>
        <button
          type="button"
          className={control}
          disabled={!state || at < 0 || at >= boards.length - 1}
          onClick={() => act(() => renderer.play.go(boards[at + 1].id))}
        >
          Next ›
        </button>
        <button
          type="button"
          className={control}
          disabled={!state || state.history.length === 0}
          onClick={() => act(() => renderer.play.back())}
        >
          ← Back
        </button>
      </div>
    </div>,
    document.body,
  );
}
