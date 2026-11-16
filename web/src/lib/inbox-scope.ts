import { useCallback, useSyncExternalStore } from "react";
import type { HomeResponse } from "@/api/generated";
import { loadMyUserId } from "@/api/members";
import { useReloading } from "@/lib/reloading";

const EVERYONE_KEY = "omni.inbox.everyone";
const listeners = new Set<() => void>();
let everyone = false;
try {
  everyone = sessionStorage.getItem(EVERYONE_KEY) === "1";
} catch {
  // No session storage: the choice lives in memory only.
}

/** The inbox switch's choice: kept in memory and in this tab's session storage, so it survives navigation and reloads. */
export function useEveryone(): [boolean, (value: boolean) => void] {
  const value = useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => everyone,
  );
  const set = useCallback((next: boolean) => {
    everyone = next;
    try {
      sessionStorage.setItem(EVERYONE_KEY, next ? "1" : "0");
    } catch {
      // Memory only.
    }
    for (const listener of listeners) listener();
  }, []);
  return [value, set];
}

/** The home with only the waiting comments on tracks the member owns (owner=me). */
function mineOf(home: HomeResponse, me: string): HomeResponse {
  const owned = new Set(
    (home.tracks ?? []).filter(({ track }) => track.owner_user_id === me).map(({ track }) => track.id),
  );
  const comments = home.waiting.comments.filter((comment) => comment.track_id && owned.has(comment.track_id));
  return { ...home, waiting: { ...home.waiting, comments } };
}

/**
 * The inbox's scope: the switch's choice, and the home narrowed to it — the
 * member's own tracks, or everyone's. Until the member is known the scoped
 * home is null, rather than flash everyone's questions.
 */
export function useInboxScope(workspaceId: string) {
  const [everyone, setEveryone] = useEveryone();
  const loadMe = useCallback(() => loadMyUserId(workspaceId), [workspaceId]);
  const { value: me } = useReloading(loadMe);
  const scoped = useCallback(
    (home: HomeResponse): HomeResponse | null => (everyone ? home : me ? mineOf(home, me) : null),
    [everyone, me],
  );
  return { everyone, setEveryone, scoped };
}
