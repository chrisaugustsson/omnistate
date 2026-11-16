import { useCallback, useSyncExternalStore } from "react";
import { useParts } from "@/lib/use-parts";

const key = (trackId: string) => `omni.showUi.${trackId}`;
const listeners = new Set<() => void>();
// The choices made in this tab, for when session storage is blocked.
const memory = new Map<string, boolean>();

function read(trackId: string | null): boolean {
  if (!trackId) return false;
  try {
    const stored = sessionStorage.getItem(key(trackId));
    if (stored !== null) return stored === "1";
  } catch {
    // Memory only.
  }
  return memory.get(trackId) ?? false;
}

/** Whether a member chose to show the UI step of a track: kept per track in this tab's session storage, and shared by every view of the track. */
export function useShowUi(trackId: string | null): [boolean, (value: boolean) => void] {
  const shown = useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => read(trackId),
  );
  const set = useCallback(
    (next: boolean) => {
      if (!trackId) return;
      memory.set(trackId, next);
      try {
        sessionStorage.setItem(key(trackId), next ? "1" : "0");
      } catch {
        // Memory only.
      }
      for (const listener of listeners) listener();
    },
    [trackId],
  );
  return [shown, set];
}

/**
 * The UI step of a track: on a service part it is hidden until the member
 * shows it; on an app part, or with no track, it always shows.
 */
export function useUiStep(workspaceId: string, projectId: string, track: { id: string; part_id: string } | undefined) {
  const { parts } = useParts(workspaceId, projectId);
  const [shown, setShown] = useShowUi(track?.id ?? null);
  const onService = !!track && parts?.find((part) => part.id === track.part_id)?.kind === "service";
  return { onService, shown, setShown, hidden: onService && !shown };
}
