import { useCallback, useEffect, useRef, useState } from "react";

/** Loads a value, and again after each call of `reload`; an answer that is no longer the newest is dropped. */
export function useReloading<T>(load: () => Promise<T>) {
  const [value, setValue] = useState<T | null>(null);
  const [failed, setFailed] = useState(false);
  const newest = useRef(0);

  const reload = useCallback(() => {
    newest.current += 1;
    const mine = newest.current;
    load().then(
      (loaded) => {
        if (mine !== newest.current) return;
        setValue(loaded);
        setFailed(false);
      },
      () => {
        if (mine === newest.current) setFailed(true);
      },
    );
  }, [load]);

  useEffect(() => {
    setValue(null);
    reload();
    return () => {
      newest.current += 1;
    };
  }, [reload]);
  return { value, failed, reload };
}

export type Reloading<T> = ReturnType<typeof useReloading<T>>;
