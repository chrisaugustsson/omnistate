import { createContext, useCallback, useContext, useLayoutEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { Section } from "./scope";

// What the pages tell the shell, and what the shell tells the pages. A level's
// layout reports the counts of its sections (useReportSections), which the
// sidebar and the phone's section button and sheet show; a page asks whether
// it is on the phone (useIsPhone); a page may put actions in the header
// (HeaderActions).

/** What the sidebar says about one section. */
export type SectionCount = {
  /** A size, shown pale ("3", "0/2"). */
  count?: number | string;
  /** How many things wait on the reader there: shown dark, in place of the size. */
  waiting?: number;
  /** A short note for the phone's sections sheet ("0 of 2", "2 boards"); the count when left out. */
  note?: string;
  /** The level lacks this section here (Design on a feature with no design docs). */
  hidden?: boolean;
};

export type SectionCounts = Partial<Record<Section, SectionCount>>;

type Reported = { key: string; counts: SectionCounts };

type Shell = {
  reported: Reported | null;
  report: (key: string, counts: SectionCounts | null) => void;
  actions: HTMLElement | null;
  setActions: (element: HTMLElement | null) => void;
};

const ShellContext = createContext<Shell | null>(null);

/** Holds what the pages report to the shell; the workspace layout wraps the shell and the pages in it. */
export function ShellProvider({ children }: { children: ReactNode }) {
  const [reported, setReported] = useState<Reported | null>(null);
  const [actions, setActions] = useState<HTMLElement | null>(null);
  const report = useCallback((key: string, counts: SectionCounts | null) => {
    setReported((now) => {
      if (counts === null) return now?.key === key ? null : now;
      if (now?.key === key && JSON.stringify(now.counts) === JSON.stringify(counts)) return now;
      return { key, counts };
    });
  }, []);
  const value = useMemo(() => ({ reported, report, actions, setActions }), [reported, report, actions]);
  return <ShellContext.Provider value={value}>{children}</ShellContext.Provider>;
}

function useShell(): Shell {
  const shell = useContext(ShellContext);
  if (!shell) throw new Error("The shell is used outside the workspace layout");
  return shell;
}

/** The key a level's counts are reported under: the level and the ID of its object. */
export function countsKey(level: string, id: string | undefined): string {
  return `${level}:${id ?? ""}`;
}

/**
 * Tells the shell the counts of the sections of the level under `key`
 * (countsKey("track", trackId)). The sidebar shows them while that level is
 * open; they are withdrawn when the reporter unmounts.
 */
export function useReportSections(key: string, counts: SectionCounts) {
  const { report } = useShell();
  const serialized = JSON.stringify(counts);
  // Before the browser paints, so a page never shows its sidebar without the counts it has.
  useLayoutEffect(() => {
    report(key, JSON.parse(serialized) as SectionCounts);
  }, [report, key, serialized]);
  useLayoutEffect(() => () => report(key, null), [report, key]);
}

/** The counts reported under `key`, or none yet. */
export function useReportedSections(key: string): SectionCounts {
  const { reported } = useShell();
  return reported?.key === key ? reported.counts : NONE;
}

const NONE: SectionCounts = {};

/** Where the header's page actions go; the header sets it. */
export function useSetHeaderActions() {
  return useShell().setActions;
}

/** Puts `children` at the right of the header, before Search (desktop), or beside the title (phone). */
export function HeaderActions({ children }: { children: ReactNode }) {
  const { actions } = useShell();
  return actions ? createPortal(children, actions) : null;
}

const PHONE = "(max-width: 767.98px)";

function subscribePhone(listener: () => void) {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const query = window.matchMedia(PHONE);
  query.addEventListener("change", listener);
  return () => query.removeEventListener("change", listener);
}

/** Whether the window is phone-wide (under 768px): the shell then shows the phone header and the bottom bar. */
export function useIsPhone(): boolean {
  return useSyncExternalStore(
    subscribePhone,
    () => typeof window !== "undefined" && !!window.matchMedia && window.matchMedia(PHONE).matches,
    () => false,
  );
}
