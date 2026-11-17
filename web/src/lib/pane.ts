import { useCallback, useEffect, useRef, useState } from "react";

/** The key of the review pane beside a doc or a design. */
export const REVIEW_PANE = "omni:pane:review";

/** Whether the pane under `key` is shown: shown until the reader hides it, and kept in localStorage. */
export function usePane(key: string): [boolean, (shown: boolean) => void] {
  const [shown, setShown] = useState(() => {
    try {
      return localStorage.getItem(key) !== "closed";
    } catch {
      return true;
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
 * stays selected does not bring back a pane the reader then hides.
 */
export function useReviewPane(
  selected: string | null,
  /** The doc's comments; `null` while they load. */
  comments: { node: string; orphaned?: boolean }[] | null,
): [boolean, (shown: boolean) => void] {
  const [shown, setShown] = usePane(REVIEW_PANE);
  const answered = useRef<string | null>(null);
  useEffect(() => {
    if (selected === null) answered.current = null;
    if (selected === null || comments === null || answered.current === selected) return;
    answered.current = selected;
    if (comments.some((on) => on.node === selected && !on.orphaned)) setShown(true);
  }, [selected, comments, setShown]);
  return [shown, setShown];
}
