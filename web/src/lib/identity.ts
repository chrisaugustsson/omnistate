// Who and what things are, as the shell draws them: the colour of a part, the
// tint and name of a person. Pure functions; the components are in
// components/shell (PartIcon, ProjectMark, Avatar).

/** How many identity colours parts have (--part-1 … --part-8 in styles.css). */
export const PART_TONES = 8;

/** How many tints people have (--person-1 … --person-6). */
export const PERSON_TONES = 6;

/**
 * The colour (1 … PART_TONES) of each part of a project: its place among the
 * project's parts in the order they were created, so a part keeps its colour
 * when parts are added, and a project's first parts never share one.
 */
export function partTones(parts: readonly { id: string; created_at: string }[]): Map<string, number> {
  const ordered = [...parts].sort((a, b) => a.created_at.localeCompare(b.created_at) || a.id.localeCompare(b.id));
  return new Map(ordered.map((part, place) => [part.id, (place % PART_TONES) + 1]));
}

/** The colour of the part `partId` among `parts`; 1 while the parts are unknown. */
export function partTone(parts: readonly { id: string; created_at: string }[] | null | undefined, partId: string): number {
  return (parts && partTones(parts).get(partId)) ?? 1;
}

/** A stable tint (1 … PERSON_TONES) for a person, from their name or ID. */
export function personTone(key: string): number {
  let hash = 0;
  for (const char of key) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return (hash % PERSON_TONES) + 1;
}

/**
 * A member's name as the UI shows it (decision U8): the part of the email
 * before the @, up to its first dot, dash, underscore or plus, capitalised.
 * "jane.doe" at any domain is "Jane".
 */
export function personName(email: string): string {
  const local = email.split("@", 1)[0] ?? "";
  const first = local.split(/[._+-]/, 1)[0] || local;
  return first ? first.charAt(0).toUpperCase() + first.slice(1) : email;
}

/** The initial an avatar or a mark shows: the first letter or digit of the name, upper case. */
export function initial(name: string): string {
  const letter = name.match(/[\p{L}\p{N}]/u)?.[0];
  return letter ? letter.toUpperCase() : "?";
}
