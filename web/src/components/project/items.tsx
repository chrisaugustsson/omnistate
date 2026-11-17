// What the project's pages share about docs: which ones sit at the project
// level, and the pillars they are grouped by.

/**
 * Whether a doc sits at the project level: on no track, feature or part, as
 * `omni doc list --global` lists them. The home holds every doc of the project.
 */
export function isProjectLevel(doc: { track_id?: string | null; feature_id?: string | null; part_id?: string | null }): boolean {
  return !doc.track_id && !doc.feature_id && !doc.part_id;
}

/** The pillars of a scope's docs, in the order the pages show them. */
export const PILLARS = [
  { pillar: "spec", title: "Spec", blurb: "What gets built and why" },
  { pillar: "ui", title: "UI", blurb: "Design system and screens" },
  { pillar: "architecture", title: "Architecture", blurb: "How the system fits together" },
  { pillar: "db", title: "DB", blurb: "The data model" },
  { pillar: "research", title: "Research", blurb: "What was learned along the way" },
];
