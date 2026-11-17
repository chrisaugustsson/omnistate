import { useCallback, useSyncExternalStore } from "react";

const EVERYONE_KEY = "omni.inbox.everyone";
const listeners = new Set<() => void>();
let everyone = false;
try {
  everyone = sessionStorage.getItem(EVERYONE_KEY) === "1";
} catch {
  // No session storage: the choice lives in memory only.
}

/** The inbox switch's choice (Mine or Everyone's): kept in memory and in this tab's session storage, so it survives navigation and reloads. */
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
