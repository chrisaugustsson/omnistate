import { useCallback, useState } from "react";

const key = (trackId: string) => `omni.showUi.${trackId}`;

/** Whether a member chose to show the UI step of a track: kept per track in this tab's session storage. */
export function useShowUi(trackId: string | null): [boolean, (value: boolean) => void] {
  const [shown, setShown] = useState(() => {
    if (!trackId) return false;
    try {
      return sessionStorage.getItem(key(trackId)) === "1";
    } catch {
      return false;
    }
  });
  const set = useCallback(
    (next: boolean) => {
      setShown(next);
      if (!trackId) return;
      try {
        sessionStorage.setItem(key(trackId), next ? "1" : "0");
      } catch {
        // Memory only.
      }
    },
    [trackId],
  );
  return [shown, set];
}
