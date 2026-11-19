import { useMatches, useParams } from "@tanstack/react-router";

// Where the reader is (navigation F, tasks/ui-redesign/build-f.md): a level of
// the path (the workspace, a project, a part, a feature or a track) and the lit
// section of that level. The path header, the sections sidebar and the phone's
// sheets all read the tables below. Routes say where they are in their
// staticData: each level's layout sets `level`, each page sets `section`, and a
// canvas page (Behaviour, Design, Board) sets `canvas`, which folds the sidebar
// to icons. The deepest match that says a thing wins.
//
//   Workspace  /w/$ws, /my-tracks, /projects                       level "workspace"
//   Project    /w/$ws/projects/$p/…                                level "project"
//   Part       /w/$ws/projects/$p/parts/$part/…                    level "part"
//   Feature    …/parts/$part/features/$f/…                         level "feature"
//   Track      /w/$ws/projects/$p/tracks/$t/… (its part from data)  level "track"

export type Level = "workspace" | "project" | "part" | "feature" | "track";

export type WorkspaceSection = "inbox" | "my-tracks" | "projects";
export type ProjectSection = "overview" | "parts" | "tracks" | "docs" | "decisions";
export type PartSection = "overview" | "features" | "tracks" | "docs" | "decisions";
export type FeatureSection = "overview" | "docs" | "design" | "tracks" | "decisions";
export type TrackSection = "spec" | "behaviour" | "design" | "how" | "board" | "threads" | "decisions";
export type Section = WorkspaceSection | ProjectSection | PartSection | FeatureSection | TrackSection;

declare module "@tanstack/react-router" {
  interface StaticDataRouteOption {
    /** The level of the path this route and the routes under it are on. */
    level?: Level;
    /** The lit section of the level. */
    section?: Section;
    /** The page is a canvas (Behaviour, Design, Board): the sidebar folds to icons on its own. */
    canvas?: boolean;
    /** The page fills the view under the header edge to edge (a viewer): no page frame, no object header. */
    bleed?: boolean;
  }
}

/** Each level's sections, in the order the sidebar and the sections sheet list them. */
export const SECTIONS = {
  workspace: ["inbox", "my-tracks", "projects"],
  project: ["overview", "parts", "tracks", "docs", "decisions"],
  part: ["overview", "features", "tracks", "docs", "decisions"],
  feature: ["overview", "docs", "design", "tracks", "decisions"],
  track: ["spec", "behaviour", "design", "how", "board", "threads", "decisions"],
} as const satisfies Record<Level, readonly Section[]>;

export const SECTION_LABELS: Record<Section, string> = {
  inbox: "Inbox",
  "my-tracks": "My tracks",
  projects: "Projects",
  overview: "Overview",
  parts: "Parts",
  features: "Features",
  tracks: "Tracks",
  docs: "Docs",
  decisions: "Decisions",
  design: "Design",
  spec: "Spec",
  behaviour: "Behaviour",
  how: "How",
  board: "Board",
  threads: "Threads",
};

/** The sections of `level`. */
export function sectionsOf(level: Level): readonly Section[] {
  return SECTIONS[level];
}

const has = <S extends Section>(list: readonly S[], section: Section | undefined): section is S =>
  section !== undefined && (list as readonly Section[]).includes(section);

/** The section a project opens on for `section`: its own, Parts for a part's Features, Overview otherwise. */
export function projectSection(section: Section | undefined): ProjectSection {
  if (section === "features") return "parts";
  return has(SECTIONS.project, section) ? section : "overview";
}

/** The section a part opens on for `section`: its own, Features for a project's Parts, Overview otherwise. */
export function partSection(section: Section | undefined): PartSection {
  if (section === "parts") return "features";
  return has(SECTIONS.part, section) ? section : "overview";
}

/** The section another feature opens on for `section`: the same one, but Overview for Design, which a feature without design docs lacks. */
export function featureSection(section: Section | undefined): FeatureSection {
  return has(SECTIONS.feature, section) && section !== "design" ? section : "overview";
}

/** The section another track opens on for `section`: the same one, Spec otherwise. */
export function trackSection(section: Section | undefined): TrackSection {
  return has(SECTIONS.track, section) ? section : "spec";
}

/** The page of a workspace's section. Its Inbox is the workspace's own page. */
export function workspaceLink(workspaceId: string, section: WorkspaceSection) {
  const params = { workspaceId };
  switch (section) {
    case "inbox":
      return { to: "/w/$workspaceId", params } as const;
    case "my-tracks":
      return { to: "/w/$workspaceId/my-tracks", params } as const;
    case "projects":
      return { to: "/w/$workspaceId/projects", params } as const;
  }
}

/** The page of a project's section. */
export function projectLink(workspaceId: string, projectId: string, section: ProjectSection) {
  const params = { workspaceId, projectId };
  switch (section) {
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

/** The page of a part's section. */
export function partLink(workspaceId: string, projectId: string, partId: string, section: PartSection) {
  const params = { workspaceId, projectId, partId };
  switch (section) {
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

/** The page of a feature's section. */
export function featureLink(workspaceId: string, projectId: string, partId: string, featureId: string, section: FeatureSection) {
  const params = { workspaceId, projectId, partId, featureId };
  switch (section) {
    case "overview":
      return { to: "/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId", params } as const;
    case "docs":
      return { to: "/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/docs", params } as const;
    case "design":
      return { to: "/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/design", params } as const;
    case "tracks":
      return { to: "/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/tracks", params } as const;
    case "decisions":
      return { to: "/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/decisions", params } as const;
  }
}

/** The page of a track's section. */
export function trackLink(workspaceId: string, projectId: string, trackId: string, section: TrackSection) {
  const params = { workspaceId, projectId, trackId };
  switch (section) {
    case "spec":
      return { to: "/w/$workspaceId/projects/$projectId/tracks/$trackId", params } as const;
    case "behaviour":
      return { to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/behaviour", params } as const;
    case "design":
      return { to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/design", params } as const;
    case "how":
      return { to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/how", params } as const;
    case "board":
      return { to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/board", params } as const;
    case "threads":
      return { to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/threads", params } as const;
    case "decisions":
      return { to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/decisions", params } as const;
  }
}

/** A place on the path: a level and the IDs it needs. A track's part is not in its URL, so it is not needed here. */
export type Place = {
  level: Level;
  workspaceId: string;
  projectId?: string;
  partId?: string;
  featureId?: string;
  trackId?: string;
};

export type SectionLink =
  | ReturnType<typeof workspaceLink>
  | ReturnType<typeof projectLink>
  | ReturnType<typeof partLink>
  | ReturnType<typeof featureLink>
  | ReturnType<typeof trackLink>;

/** The page of `section` at `place`; a section the level does not have opens the level's first. */
export function sectionLink(place: Place, section: Section | undefined): SectionLink {
  const { workspaceId, projectId = "", partId = "", featureId = "", trackId = "" } = place;
  switch (place.level) {
    case "workspace":
      return workspaceLink(workspaceId, has(SECTIONS.workspace, section) ? section : "inbox");
    case "project":
      return projectLink(workspaceId, projectId, has(SECTIONS.project, section) ? section : "overview");
    case "part":
      return partLink(workspaceId, projectId, partId, has(SECTIONS.part, section) ? section : "overview");
    case "feature":
      return featureLink(workspaceId, projectId, partId, featureId, has(SECTIONS.feature, section) ? section : "overview");
    case "track":
      return trackLink(workspaceId, projectId, trackId, has(SECTIONS.track, section) ? section : "spec");
  }
}

export type Scope = Place & {
  /** The lit section; undefined on a page of the level that is none of its sections (API keys, a doc opened full view). */
  section: Section | undefined;
  canvas: boolean;
  bleed: boolean;
};

/** Where the current page is, from the matched routes' staticData and the URL's params. */
export function useScope(): Scope {
  const said = useMatches({
    select: (matches) => {
      let level: Level = "workspace";
      let section: Section | undefined;
      let canvas = false;
      let bleed = false;
      for (const match of matches) {
        const data = match.staticData;
        if (data.level) {
          level = data.level;
          section = undefined;
        }
        if (data.section) section = data.section;
        if (data.canvas) canvas = true;
        if (data.bleed) bleed = true;
      }
      return { level, section, canvas, bleed };
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
