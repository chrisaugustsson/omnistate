// The difference between two design token trees (DTCG JSON, with a value per
// mode in `$extensions.omnistate.modes`, docs/formats/tokens.md), token by
// token, for the change set review.

/** One token's values: its `$value` and its value in each mode it has. */
export type TokenValues = { value: unknown; modes: Record<string, unknown> };

export type TokenChange = {
  /** The token's path, its group names and its name joined with dots: "color.accent". */
  path: string;
  change: "added" | "changed" | "removed";
  before: TokenValues | null;
  after: TokenValues | null;
};

type Json = Record<string, unknown>;

function isObject(value: unknown): value is Json {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Every token of `tree` by its path; groups are the objects without a `$value`. */
export function flattenTokens(tree: Json, prefix = "", into = new Map<string, TokenValues>()): Map<string, TokenValues> {
  for (const [key, child] of Object.entries(tree)) {
    if (key.startsWith("$") || !isObject(child)) continue;
    const path = prefix ? `${prefix}.${key}` : key;
    if ("$value" in child) {
      const extensions = isObject(child.$extensions) ? child.$extensions : {};
      const omnistate = isObject(extensions.omnistate) ? extensions.omnistate : {};
      const modes = isObject(omnistate.modes) ? omnistate.modes : {};
      into.set(path, { value: child.$value, modes });
    } else {
      flattenTokens(child, path, into);
    }
  }
  return into;
}

function same(a: TokenValues, b: TokenValues): boolean {
  return JSON.stringify([a.value, a.modes]) === JSON.stringify([b.value, b.modes]);
}

/** The tokens added, changed and removed from `base` (none for a new design system) to `next`, in path order. */
export function diffTokens(base: Json | null, next: Json): TokenChange[] {
  const before = base ? flattenTokens(base) : new Map<string, TokenValues>();
  const after = flattenTokens(next);
  const paths = [...new Set([...before.keys(), ...after.keys()])].sort();
  const changes: TokenChange[] = [];
  for (const path of paths) {
    const old = before.get(path) ?? null;
    const now = after.get(path) ?? null;
    if (old && now) {
      if (!same(old, now)) changes.push({ path, change: "changed", before: old, after: now });
    } else if (now) {
      changes.push({ path, change: "added", before: null, after: now });
    } else if (old) {
      changes.push({ path, change: "removed", before: old, after: null });
    }
  }
  return changes;
}
