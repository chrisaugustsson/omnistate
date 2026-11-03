/** The design-system head is a DTCG token tree plus reusable components. */
export type JsonObject = Record<string, unknown>;
export type SystemToken = {
  path: string;
  group: string;
  type: string;
  description: string;
  definition: JsonObject;
};

export function object(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

export function readSystem(head: string): JsonObject {
  if (!head.trim()) return {};
  const parsed: unknown = JSON.parse(head);
  if (!object(parsed)) throw new Error("The design system could not be read.");
  return parsed;
}

export function systemModes(system: JsonObject): string[] {
  const extensions = system.$extensions;
  const omni = object(extensions) ? extensions.omnistate : null;
  const modes = object(omni) ? omni.modes : null;
  return Array.isArray(modes) ? modes.filter((mode): mode is string => typeof mode === "string") : [];
}

export function systemTokens(system: JsonObject): SystemToken[] {
  const found: SystemToken[] = [];
  const visit = (node: JsonObject, path: string[], inheritedType: string) => {
    const type = typeof node.$type === "string" ? node.$type : inheritedType;
    if ("$value" in node) {
      found.push({
        path: path.join("."),
        group: path[0],
        type,
        description: typeof node.$description === "string" ? node.$description : "",
        definition: node,
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

/** Resolves whole-value and composite aliases in the selected mode, with a cycle guard. */
export function tokenValues(tokens: SystemToken[], mode: string | null): Map<string, unknown> {
  const byPath = new Map(tokens.map((token) => [token.path, token]));
  const resolved = new Map<string, unknown>();
  const resolve = (path: string, visiting: Set<string>): unknown => {
    if (resolved.has(path)) return resolved.get(path);
    const token = byPath.get(path);
    if (!token || visiting.has(path)) throw new Error(`Unresolved reference: ${path}`);
    const next = new Set(visiting).add(path);
    const value = expand(tokenSourceValue(token, mode), next);
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
