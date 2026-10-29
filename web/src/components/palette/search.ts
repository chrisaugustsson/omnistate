import type { HomeResponse } from "@/api/generated";
import { PILLARS } from "@/components/project/items";
import type { Visit } from "@/lib/visits";

// The palette's search: pure functions over the project home. Items are found
// by feature name, doc title and thread question, never by what a doc says.

export const MOST_FEATURES = 5;
export const MOST_DOCS = 6;
export const MOST_THREADS = 3;
export const MOST_RECENT = 6;

export type PlaceKind = "app" | "feature" | "doc" | "thread";

/** A place the palette can open: its name, and the scope and pillar it is in. */
export type Place = {
  kind: PlaceKind;
  /** The feature, doc or thread ID; "app" for the App page. */
  id: string;
  name: string;
  /** "Checkout · UI" for a doc, the scope's name for a thread, empty for a scope. */
  context: string;
};

type Entry = Place & { lower: string; at: number };

/** A group of results: the first few matches, and how many there were. */
export type Group = { items: Place[]; total: number };

export type Index = {
  features: Entry[];
  docs: Entry[];
  threads: Entry[];
  places: Map<string, Entry>;
};

export type Results = { features: Group; docs: Group; threads: Group };

/** The query as the palette and the Features page read it: trimmed and lower case. */
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
 * What the palette searches, from the project home. A feature's activity is its
 * `last_activity` (its creation while it has none), a doc's its `updated_at`,
 * and a thread's its newest post (its creation while it has none). Each list is
 * ordered by activity here, so that a search is one pass over it.
 */
export function buildIndex(home: HomeResponse): Index {
  const scopes = new Map<string, string>([["app", "App"]]);
  const places = new Map<string, Entry>();
  const add = (list: Entry[], entry: Entry) => {
    list.push(entry);
    places.set(entryKey(entry.kind, entry.id), entry);
  };
  const entry = (kind: PlaceKind, id: string, name: string, context: string, at: number): Entry => ({
    kind,
    id,
    name,
    context,
    lower: name.toLowerCase(),
    at,
  });

  const features: Entry[] = [];
  for (const { feature, last_activity } of home.features ?? []) {
    scopes.set(feature.id, feature.name);
    add(features, entry("feature", feature.id, feature.name, "", time(last_activity, feature.created_at)));
  }
  const docs: Entry[] = [];
  for (const doc of home.docs) {
    const scope = scopes.get(doc.feature_id ?? "app") ?? "";
    const pillar = PILLAR_TITLES.get(doc.pillar) ?? doc.pillar;
    add(docs, entry("doc", doc.id, doc.title, `${scope} · ${pillar}`, time(doc.updated_at, doc.created_at)));
  }
  const threads: Entry[] = [];
  for (const thread of home.threads) {
    const scope = scopes.get(thread.feature_id ?? "app") ?? "";
    add(threads, entry("thread", thread.id, thread.question, scope, time(thread.last_post_at, thread.created_at)));
  }
  places.set(entryKey("app", "app"), entry("app", "app", "App", "", 0));
  for (const list of [features, docs, threads]) list.sort(byRecent);
  return { features, docs, threads, places };
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
    features: find(index.features, query, MOST_FEATURES),
    docs: find(index.docs, query, MOST_DOCS),
    threads: find(index.threads, query, MOST_THREADS),
  };
}

/** The places of the newest visits that the project still has: the App page, features, docs and threads. */
export function recentPlaces(index: Index, visits: Visit[], most = MOST_RECENT): Place[] {
  const places: Place[] = [];
  for (const visit of visits) {
    const found = visit.doc
      ? index.places.get(entryKey("doc", visit.doc))
      : visit.thread
        ? index.places.get(entryKey("thread", visit.thread))
        : index.places.get(entryKey(visit.scope === "app" ? "app" : "feature", visit.scope));
    if (found) places.push(found);
    if (places.length === most) break;
  }
  return places;
}
