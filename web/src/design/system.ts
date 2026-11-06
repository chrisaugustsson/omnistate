/** The design-system head is a DTCG token tree plus reusable components. */
export type JsonObject = Record<string, unknown>;
export type SystemToken = {
  path: string;
  group: string;
  type: string;
  description: string;
  definition: JsonObject;
  scopes?: string[];
  codeSyntax?: Record<string, string>;
};
export type Collection = { name: string; modes: string[] };

export function object(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

export function readSystem(head: string): JsonObject {
  if (!head.trim()) return {};
  const parsed: unknown = JSON.parse(head);
  if (!object(parsed)) throw new Error("The design system could not be read.");
  return parsed;
}

function modeList(node: JsonObject): string[] | null {
  const extensions = node.$extensions;
  const omni = object(extensions) ? extensions.omnistate : null;
  const modes = object(omni) ? omni.modes : null;
  return Array.isArray(modes) ? modes.filter((mode): mode is string => typeof mode === "string") : null;
}

/** The top-level groups that hold tokens, in tree order; a group without modes of its own takes the root list. */
export function collectionsOf(system: JsonObject): Collection[] {
  const root = modeList(system) ?? [];
  return [...new Set(systemTokens(system).map((token) => token.group))].map((name) => {
    const group = system[name];
    return { name, modes: (object(group) ? modeList(group) : null) ?? root };
  });
}

export function systemTokens(system: JsonObject): SystemToken[] {
  const found: SystemToken[] = [];
  const visit = (node: JsonObject, path: string[], inheritedType: string) => {
    const type = typeof node.$type === "string" ? node.$type : inheritedType;
    if ("$value" in node) {
      const extensions = node.$extensions;
      const omni = object(extensions) && object(extensions.omnistate) ? extensions.omnistate : {};
      const scopes = Array.isArray(omni.scopes) ? omni.scopes.filter((scope): scope is string => typeof scope === "string") : undefined;
      const codeSyntax = object(omni.codeSyntax)
        ? Object.fromEntries(Object.entries(omni.codeSyntax).filter((entry): entry is [string, string] => typeof entry[1] === "string"))
        : undefined;
      found.push({
        path: path.join("."),
        group: path[0],
        type,
        description: typeof node.$description === "string" ? node.$description : "",
        definition: node,
        ...(scopes && { scopes }),
        ...(codeSyntax && { codeSyntax }),
      });
      return;
    }
    for (const [name, child] of Object.entries(node)) {
      if (!name.startsWith("$") && object(child)) visit(child, [...path, name], type);
    }
  };
  visit(system, [], "");
  return found;
}

/** The selected mode's source value, before aliases are expanded. */
export function tokenSourceValue(token: SystemToken, mode: string | null): unknown {
  const extensions = token.definition.$extensions;
  const omni = object(extensions) ? extensions.omnistate : null;
  const modes = object(omni) && object(omni.modes) ? omni.modes : {};
  return mode !== null && Object.hasOwn(modes, mode) ? modes[mode] : token.definition.$value;
}

/** Resolves whole-value and composite aliases, each token in the mode chosen for its collection, with a cycle guard. */
export function tokenValues(tokens: SystemToken[], choice: Record<string, string>): Map<string, unknown> {
  const byPath = new Map(tokens.map((token) => [token.path, token]));
  const resolved = new Map<string, unknown>();
  const resolve = (path: string, visiting: Set<string>): unknown => {
    if (resolved.has(path)) return resolved.get(path);
    const token = byPath.get(path);
    if (!token || visiting.has(path)) throw new Error(`Unresolved reference: ${path}`);
    const next = new Set(visiting).add(path);
    const value = expand(tokenSourceValue(token, choice[token.group] ?? null), next);
    resolved.set(path, value);
    return value;
  };
  const expand = (value: unknown, visiting: Set<string>): unknown => {
    if (typeof value === "string") {
      const reference = /^\{([^{}]+)\}$/.exec(value);
      return reference ? resolve(reference[1], visiting) : value;
    }
    if (Array.isArray(value)) return value.map((part) => expand(part, visiting));
    if (object(value)) return Object.fromEntries(Object.entries(value).map(([key, part]) => [key, expand(part, visiting)]));
    return value;
  };
  for (const token of tokens) {
    try {
      resolve(token.path, new Set());
    } catch {
      resolved.set(token.path, null);
    }
  }
  return resolved;
}

export function systemComponents(system: JsonObject): JsonObject[] {
  return Array.isArray(system.$components)
    ? system.$components.filter((component): component is JsonObject => object(component) && typeof component.id === "string")
    : [];
}

export function componentLabel(component: JsonObject): string {
  const variants = object(component.props)
    ? Object.values(component.props).filter((prop) => object(prop) && prop.type === "variant").map((prop) => String((prop as JsonObject).default ?? ""))
    : [];
  return [String(component.name ?? "Component"), ...variants.filter(Boolean)].join(" / ");
}
