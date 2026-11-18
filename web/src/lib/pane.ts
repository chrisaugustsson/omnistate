import { useCallback, useEffect, useRef, useState } from "react";

/** The key of the review pane beside a doc or a design. */
export const REVIEW_PANE = "omni:pane:review";
/** The key of the review pane of a doc shown inside a track's tab, beside the track's panel. */
export const EMBEDDED_REVIEW_PANE = "omni:pane:review:embedded";

/** Below this window width a design's canvas needs the room: its review pane starts hidden on a Docs tab. */
export const NARROW_FOR_REVIEW = 1440;

/**
 * Whether the pane under `key` is shown: `shownByDefault` until the reader
 * shows or hides it, and the reader's choice is kept in localStorage. With
 * `startHidden` it starts hidden whatever was kept, and shows once asked to.
 */
export function usePane(key: string, shownByDefault = true, startHidden = false): [boolean, (shown: boolean) => void] {
  const [shown, setShown] = useState(() => {
    if (startHidden) return false;
    try {
      const kept = localStorage.getItem(key);
      return kept === null ? shownByDefault : kept !== "closed";
    } catch {
      return shownByDefault;
    }
  });
  const set = useCallback(
    (next: boolean) => {
      setShown(next);
      try {
        localStorage.setItem(key, next ? "open" : "closed");
      } catch {
        // Storage that is blocked only costs the kept choice.
      }
    },
    [key],
  );
  return [shown, set];
}

/**
 * The review pane beside a viewer. A reader who newly selects something that
 * has comments asked for its discussion, so a hidden pane is shown again; what
 * stays selected does not bring back a pane the reader then hides. Inside a
 * track's tab (`embedded`) the pane starts hidden, so the doc keeps its width
 * beside the track's panel; with `narrow` (a design on a Docs tab in a window
 * under NARROW_FOR_REVIEW) it starts hidden too, so the canvas keeps its room.
 */
export function useReviewPane(
  selected: string | null,
  /** The doc's comments; `null` while they load. */
  comments: { node: string; orphaned?: boolean }[] | null,
  embedded = false,
  narrow = false,
): [boolean, (shown: boolean) => void] {
  const [shown, setShown] = usePane(embedded ? EMBEDDED_REVIEW_PANE : REVIEW_PANE, !embedded, narrow);
  const answered = useRef<string | null>(null);
  useEffect(() => {
    if (selected === null) answered.current = null;
    if (selected === null || comments === null || answered.current === selected) return;
    answered.current = selected;
    if (comments.some((on) => on.node === selected && !on.orphaned)) setShown(true);
  }, [selected, comments, setShown]);
  return [shown, setShown];
}
