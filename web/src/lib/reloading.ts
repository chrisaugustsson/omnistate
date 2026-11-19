import { useCallback, useEffect, useRef, useState } from "react";

// What each `key` loaded last, kept for this tab, so a page that shows the
// same thing again starts from it instead of from nothing, and the reload that
// follows replaces it. Moving between pages then does not flash empty lists
// and counts for a frame.
const kept = new Map<string, unknown>();

/** Forgets everything kept: on signing out, and between tests. */
export function forgetLoaded() {
  kept.clear();
}

/**
 * Loads a value, and again after each call of `reload`; an answer that is no
 * longer the newest is dropped. With a `key` (naming what is loaded, with the
 * IDs it depends on) the last value loaded under that key is shown at once
 * until the new one arrives.
 */
export function useReloading<T>(load: () => Promise<T>, key?: string) {
  const [state, setState] = useState<{ key: string | undefined; value: T | null }>(() => ({ key, value: keptUnder<T>(key) }));
  const [failed, setFailed] = useState(false);
  const newest = useRef(0);

  const reload = useCallback(() => {
    newest.current += 1;
    const mine = newest.current;
    load().then(
      (loaded) => {
        if (mine !== newest.current) return;
        if (key !== undefined && loaded !== null && loaded !== undefined) kept.set(key, loaded);
        setState({ key, value: loaded });
        setFailed(false);
      },
      () => {
        if (mine === newest.current) setFailed(true);
      },
    );
  }, [load, key]);

  useEffect(() => {
    setState({ key, value: keptUnder<T>(key) });
    reload();
    return () => {
      newest.current += 1;
    };
  }, [reload, key]);
  // Right after the key changed, before the effect runs: what is kept under the new key, never the old key's value.
  const value = state.key === key ? state.value : keptUnder<T>(key);
  return { value, failed, reload };
}

function keptUnder<T>(key: string | undefined): T | null {
  return key === undefined ? null : ((kept.get(key) as T | undefined) ?? null);
}

export type Reloading<T> = ReturnType<typeof useReloading<T>>;
