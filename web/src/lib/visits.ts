import { useEffect } from "react";

/** A place the reviewer visited: its scope ("app" or a feature's ID), and the doc or thread when there is one. */
export type Visit = { scope: string; doc?: string; thread?: string; at: number };

const MOST_VISITS = 50;

const keyOf = (projectId: string) => `omni:visits:${projectId}`;

/** The project's visits, newest first; empty when none are stored or the stored value is unusable. */
export function readVisits(projectId: string): Visit[] {
  try {
    const stored = JSON.parse(localStorage.getItem(keyOf(projectId)) ?? "[]") as unknown;
    return Array.isArray(stored) ? (stored as Visit[]) : [];
  } catch {
    return [];
  }
}

/**
 * Puts a visit at the front of the project's history with the current time. The
 * same place (scope, doc and thread) is removed from where it was, so each place
 * appears once, and only the newest 50 places are kept.
 */
export function recordVisit(projectId: string, place: Omit<Visit, "at">): void {
  const same = (visit: Visit) =>
    visit.scope === place.scope && visit.doc === place.doc && visit.thread === place.thread;
  const visit: Visit = { scope: place.scope, at: Date.now() };
  if (place.doc) visit.doc = place.doc;
  if (place.thread) visit.thread = place.thread;
  const visits = [visit, ...readVisits(projectId).filter((earlier) => !same(earlier))].slice(0, MOST_VISITS);
  try {
    localStorage.setItem(keyOf(projectId), JSON.stringify(visits));
  } catch {
    // Storage that is full or blocked only costs the history.
  }
}

/** Records the place once its scope is known (`scope` is undefined while the page's data loads or when it failed). */
export function useVisit(projectId: string, scope: string | undefined, doc?: string, thread?: string): void {
  useEffect(() => {
    if (scope) recordVisit(projectId, { scope, doc, thread });
  }, [projectId, scope, doc, thread]);
}
