import type { HomeResponse } from "@/api/generated";
import { PILLARS } from "@/components/project/items";
import type { Visit } from "@/lib/visits";

// The palette's search: pure functions over the project home, or over every
// project's home on Home (mergeIndexes). Items are found by track name, doc
// title and thread question, never by what a doc says.

export const MOST_TRACKS = 5;
export const MOST_DOCS = 6;
export const MOST_THREADS = 3;
export const MOST_RECENT = 6;

export type PlaceKind = "app" | "track" | "doc" | "thread";

/** A place the palette can open: its name, and the scope and pillar it is in. */
export type Place = {
  kind: PlaceKind;
  /** The track, doc or thread ID; "app" for the project's Overview. */
  id: string;
  /** The project the place is in. */
  projectId: string;
  name: string;
  /** "Checkout · UI" for a doc (its track, part or project), the scope's name for a thread, empty for a scope. */
  context: string;
};

type Entry = Place & { lower: string; at: number };

/** A group of results: the first few matches, and how many there were. */
export type Group = { items: Place[]; total: number };

export type Index = {
  tracks: Entry[];
  docs: Entry[];
  threads: Entry[];
  places: Map<string, Entry>;
  /** Each project's Overview, by project ID. */
  apps: Map<string, Entry>;
};

export type Results = { tracks: Group; docs: Group; threads: Group };

/** The query as the palette and the Tracks page read it: trimmed and lower case. */
export function normalize(query: string): string {
  return query.trim().toLowerCase();
}

/** Whether any of `texts` contains the query, ignoring case; an empty query matches. */
export function matches(query: string, ...texts: string[]): boolean {
  const wanted = normalize(query);
  return texts.some((text) => text.toLowerCase().includes(wanted));
}

const PILLAR_TITLES = new Map(PILLARS.map(({ pillar, title }) => [pillar, title]));

function time(iso: string | null | undefined, fallback: string): number {
  const at = Date.parse(iso ?? fallback);
  return Number.isNaN(at) ? 0 : at;
}

/** Most recent first, ties by name. */
function byRecent(a: Entry, b: Entry): number {
  return b.at - a.at || (a.lower < b.lower ? -1 : a.lower > b.lower ? 1 : 0);
}

const entryKey = (kind: PlaceKind, id: string) => `${kind}:${id}`;

/**
 * The names a place's context is written with: the project's, its parts' by
 * part ID, and the part each feature is on by feature ID. A doc of the project
 * is labelled with the project's name, a doc of a part or of one of its
 * features with the part's name, a doc of a track with the track's.
 */
export type ScopeNames = {
  project?: string;
  parts?: ReadonlyMap<string, string>;
  /** The name of the part each feature is on, by feature ID. */
  featureParts?: ReadonlyMap<string, string>;
};

/**
 * What the palette searches, from the project home. A track's activity is its
 * `last_activity` (its creation while it has none), a doc's its `updated_at`,
 * and a thread's its newest post (its creation while it has none). Each list is
 * ordered by activity here, so that a search is one pass over it.
 */
export function buildIndex(home: HomeResponse, names: ScopeNames = {}): Index {
  const project = names.project ?? "";
  const scopes = new Map<string, string>([["app", project]]);
  const places = new Map<string, Entry>();
  const add = (list: Entry[], entry: Entry) => {
    list.push(entry);
    places.set(entryKey(entry.kind, entry.id), entry);
  };
  const entry = (kind: PlaceKind, id: string, name: string, context: string, at: number): Entry => ({
    kind,
    id,
    projectId: home.project_id,
    name,
    context,
    lower: name.toLowerCase(),
    at,
  });

  const tracks: Entry[] = [];
  for (const { track, last_activity } of home.tracks ?? []) {
    scopes.set(track.id, track.name);
    add(tracks, entry("track", track.id, track.name, "", time(last_activity, track.created_at)));
  }
  const docs: Entry[] = [];
  for (const doc of home.docs) {
    const scope = doc.track_id
      ? scopes.get(doc.track_id)
      : doc.part_id
        ? names.parts?.get(doc.part_id)
        : doc.feature_id
          ? names.featureParts?.get(doc.feature_id)
          : project;
    const pillar = PILLAR_TITLES.get(doc.pillar) ?? doc.pillar;
    add(docs, entry("doc", doc.id, doc.title, scope ? `${scope} · ${pillar}` : pillar, time(doc.updated_at, doc.created_at)));
  }
  const threads: Entry[] = [];
  for (const thread of home.threads) {
    const scope = scopes.get(thread.track_id ?? "app") ?? "";
    add(threads, entry("thread", thread.id, thread.question, scope, time(thread.last_post_at, thread.created_at)));
  }
  const apps = new Map([[home.project_id, entry("app", "app", project || "Overview", "", 0)]]);
  for (const list of [tracks, docs, threads]) list.sort(byRecent);
  return { tracks, docs, threads, places, apps };
}

/** One index over several projects' indexes, each list again ordered by activity. */
export function mergeIndexes(indexes: readonly Index[]): Index {
  const merged: Index = { tracks: [], docs: [], threads: [], places: new Map(), apps: new Map() };
  for (const index of indexes) {
    merged.tracks.push(...index.tracks);
    merged.docs.push(...index.docs);
    merged.threads.push(...index.threads);
    for (const [key, place] of index.places) merged.places.set(key, place);
    for (const [key, app] of index.apps) merged.apps.set(key, app);
  }
  for (const list of [merged.tracks, merged.docs, merged.threads]) list.sort(byRecent);
  return merged;
}

function find(list: Entry[], query: string, most: number): Group {
  const wanted = normalize(query);
  const starts: Entry[] = [];
  const others: Entry[] = [];
  for (const entry of list) {
    const at = entry.lower.indexOf(wanted);
    if (at === 0) starts.push(entry);
    else if (at > 0) others.push(entry);
  }
  return { items: [...starts, ...others].slice(0, most), total: starts.length + others.length };
}

/**
 * The matches of a query in each group. An item matches when its name contains
 * the trimmed query, ignoring case; those whose name starts with it come first,
 * each part most recent first.
 */
export function search(index: Index, query: string): Results {
  return {
    tracks: find(index.tracks, query, MOST_TRACKS),
    docs: find(index.docs, query, MOST_DOCS),
    threads: find(index.threads, query, MOST_THREADS),
  };
}

/**
 * The places of the newest visits that the project still has: its Overview,
 * tracks, docs and threads. A visit from a merged index names its `project`, so
 * that its Overview is that project's.
 */
export function recentPlaces(index: Index, visits: (Visit & { project?: string })[], most = MOST_RECENT): Place[] {
  const places: Place[] = [];
  for (const visit of visits) {
    const found = visit.doc
      ? index.places.get(entryKey("doc", visit.doc))
      : visit.thread
        ? index.places.get(entryKey("thread", visit.thread))
        : visit.scope === "app"
          ? visit.project
            ? index.apps.get(visit.project)
            : index.apps.values().next().value
          : index.places.get(entryKey("track", visit.scope));
    if (found) places.push(found);
    if (places.length === most) break;
  }
  return places;
}
