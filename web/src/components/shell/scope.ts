import { useMatches, useParams } from "@tanstack/react-router";

// Where the reader is, as the top bar shows it: a scope (Home, a project, a
// part) and the lit section (pill) of that scope. Routes say it in their
// staticData: each scope's layout route sets `scope`, each page sets `tab`, and
// an object (a feature or a track) keeps its part in the bar with its section
// lit (features, tracks). The deepest match that says a thing wins.
//
//   Home      /w/$ws, /inbox, /my-tracks                     scope "home"
//   Project   /w/$ws/projects/$p/…                           scope "project"
//   Part      /w/$ws/projects/$p/parts/$part/…               scope "part"
//   Feature   …/parts/$part/features/$f/…  (bar: its part)   scope "feature", tab "features"
//   Track     /w/$ws/projects/$p/tracks/$t/… (bar: its part) scope "track", tab "tracks"

export type ScopeKind = "home" | "project" | "part" | "feature" | "track";

export type HomeTab = "for-you" | "inbox" | "my-tracks";
export type ProjectTab = "overview" | "parts" | "docs" | "tracks" | "decisions";
export type PartTab = "overview" | "features" | "docs" | "tracks" | "decisions";
export type Tab = HomeTab | ProjectTab | PartTab;

declare module "@tanstack/react-router" {
  interface StaticDataRouteOption {
    /** The scope the bar shows for this route and the routes under it. */
    scope?: ScopeKind;
    /** The lit pill of the scope. */
    tab?: Tab;
    /** The page fills the view under the bar edge to edge (a viewer, the Behaviour screen): no page frame, no object header. */
    bleed?: boolean;
  }
}

export const HOME_TABS: readonly HomeTab[] = ["for-you", "inbox", "my-tracks"];
export const PROJECT_TABS: readonly ProjectTab[] = ["overview", "parts", "docs", "tracks", "decisions"];
export const PART_TABS: readonly PartTab[] = ["overview", "features", "docs", "tracks", "decisions"];

export const TAB_LABELS: Record<Tab, string> = {
  "for-you": "For you",
  inbox: "Inbox",
  "my-tracks": "My tracks",
  overview: "Overview",
  parts: "Parts",
  features: "Features",
  docs: "Docs",
  tracks: "Tracks",
  decisions: "Decisions",
};

/** The tab a project shows for `tab`: its own, Parts for a part's Features, Overview for a Home tab. */
export function projectTab(tab: Tab | undefined): ProjectTab {
  if (tab === "features") return "parts";
  return tab && (PROJECT_TABS as readonly string[]).includes(tab) ? (tab as ProjectTab) : "overview";
}

/** The tab a part shows for `tab`: its own, Features for a project's Parts, Overview for a Home tab. */
export function partTab(tab: Tab | undefined): PartTab {
  if (tab === "parts") return "features";
  return tab && (PART_TABS as readonly string[]).includes(tab) ? (tab as PartTab) : "overview";
}

/** The page of a Home tab. */
export function homeLink(workspaceId: string, tab: HomeTab) {
  const params = { workspaceId };
  switch (tab) {
    case "for-you":
      return { to: "/w/$workspaceId", params } as const;
    case "inbox":
      return { to: "/w/$workspaceId/inbox", params } as const;
    case "my-tracks":
      return { to: "/w/$workspaceId/my-tracks", params } as const;
  }
}

/** The page of a project's tab. */
export function projectLink(workspaceId: string, projectId: string, tab: ProjectTab) {
  const params = { workspaceId, projectId };
  switch (tab) {
    case "overview":
      return { to: "/w/$workspaceId/projects/$projectId", params } as const;
    case "parts":
      return { to: "/w/$workspaceId/projects/$projectId/parts", params } as const;
    case "docs":
      return { to: "/w/$workspaceId/projects/$projectId/docs", params } as const;
    case "tracks":
      return { to: "/w/$workspaceId/projects/$projectId/tracks", params } as const;
    case "decisions":
      return { to: "/w/$workspaceId/projects/$projectId/decisions", params } as const;
  }
}

/** The page of a part's tab. */
export function partLink(workspaceId: string, projectId: string, partId: string, tab: PartTab) {
  const params = { workspaceId, projectId, partId };
  switch (tab) {
    case "overview":
      return { to: "/w/$workspaceId/projects/$projectId/parts/$partId", params } as const;
    case "features":
      return { to: "/w/$workspaceId/projects/$projectId/parts/$partId/features", params } as const;
    case "docs":
      return { to: "/w/$workspaceId/projects/$projectId/parts/$partId/docs", params } as const;
    case "tracks":
      return { to: "/w/$workspaceId/projects/$projectId/parts/$partId/tracks", params } as const;
    case "decisions":
      return { to: "/w/$workspaceId/projects/$projectId/parts/$partId/decisions", params } as const;
  }
}

export type Scope = {
  kind: ScopeKind;
  /** The lit pill; undefined on a page of the scope that is none of its pills (API keys, a doc's activity). */
  tab: Tab | undefined;
  bleed: boolean;
  workspaceId: string;
  projectId?: string;
  /** The part in the URL (part, feature); a track's part comes from its data. */
  partId?: string;
  featureId?: string;
  trackId?: string;
};

/** The scope of the current page, from the matched routes' staticData and the URL's params. */
export function useScope(): Scope {
  const said = useMatches({
    select: (matches) => {
      let kind: ScopeKind = "home";
      let tab: Tab | undefined;
      let bleed = false;
      for (const match of matches) {
        const data = match.staticData;
        if (data.scope) kind = data.scope;
        if (data.tab) tab = data.tab;
        if (data.bleed) bleed = true;
      }
      return { kind, tab, bleed };
    },
    structuralSharing: true,
  });
  const params = useParams({ strict: false }) as {
    workspaceId?: string;
    projectId?: string;
    partId?: string;
    featureId?: string;
    trackId?: string;
  };
  return {
    ...said,
    workspaceId: params.workspaceId ?? "",
    projectId: params.projectId,
    partId: params.partId,
    featureId: params.featureId,
    trackId: params.trackId,
  };
}

/**
 * Where a doc opens: a track's doc under its track, a part's under the part's
 * Docs, a project-level doc under the project's Docs. A feature's doc goes by
 * the project's URL, which sends it on to its feature (the doc does not name
 * the feature's part).
 */
export function docLink(
  workspaceId: string,
  projectId: string,
  doc: { id: string; track_id?: string | null; part_id?: string | null; feature_id?: string | null },
) {
  const params = { workspaceId, projectId, docId: doc.id };
  if (doc.track_id) {
    return { to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/docs/$docId", params: { ...params, trackId: doc.track_id } } as const;
  }
  if (doc.part_id && !doc.feature_id) {
    return { to: "/w/$workspaceId/projects/$projectId/parts/$partId/docs/$docId", params: { ...params, partId: doc.part_id } } as const;
  }
  return { to: "/w/$workspaceId/projects/$projectId/docs/$docId", params } as const;
}

/** Where a thread opens: a track's thread in the track's Threads tab; a thread of the project itself at the project's URL. */
export function threadLink(workspaceId: string, projectId: string, thread: { id: string; track_id?: string | null }) {
  const params = { workspaceId, projectId, threadId: thread.id };
  return thread.track_id
    ? ({ to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/threads/$threadId", params: { ...params, trackId: thread.track_id } } as const)
    : ({ to: "/w/$workspaceId/projects/$projectId/threads/$threadId", params } as const);
}
