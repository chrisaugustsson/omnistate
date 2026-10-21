// Turns the OpenAPI document of the API (web/openapi.json) into src/api/generated.ts:
// a Zod schema and a TypeScript type for every component schema, and the Zod
// schemas of every operation's JSON request body and JSON responses. The
// generated module is committed; `pnpm generate` rewrites it and
// src/api/generated.test.ts fails when it is out of date.

type Schema = {
  $ref?: string;
  type?: string | string[];
  format?: string;
  enum?: unknown[];
  const?: unknown;
  properties?: Record<string, Schema>;
  required?: string[];
  additionalProperties?: boolean | Schema;
  items?: Schema;
  oneOf?: Schema[];
  anyOf?: Schema[];
  allOf?: Schema[];
};

type Operation = {
  requestBody?: { content?: Record<string, { schema?: Schema }> };
  responses?: Record<string, { content?: Record<string, { schema?: Schema }> }>;
};

type Document = {
  paths: Record<string, Record<string, unknown>>;
  components?: { schemas?: Record<string, Schema> };
};

const HTTP_METHODS = ["get", "put", "post", "delete", "patch", "head", "options", "trace"];
const REF_PREFIX = "#/components/schemas/";
const JSON_CONTENT = "application/json";

function refName(ref: string): string {
  if (!ref.startsWith(REF_PREFIX)) throw new Error(`unsupported $ref ${ref}`);
  return ref.slice(REF_PREFIX.length);
}

function quote(value: string): string {
  return JSON.stringify(value);
}

function indented(lines: string[], depth: number): string {
  if (lines.length === 0) return "";
  const pad = "  ".repeat(depth + 1);
  return `\n${lines.map((line) => `${pad}${line},`).join("\n")}\n${"  ".repeat(depth)}`;
}

/** The Zod expression of a schema; `depth` is how far the expression is indented. */
function expression(schema: Schema, depth: number): string {
  if (schema.$ref) return `${refName(schema.$ref)}Schema`;
  if (schema.allOf) {
    return schema.allOf
      .map((member) => expression(member, depth))
      .reduce((left, right) => `z.intersection(${left}, ${right})`);
  }
  const alternatives = schema.oneOf ?? schema.anyOf;
  if (alternatives) {
    const members = alternatives.map((member) => expression(member, depth + 1));
    return members.length === 1
      ? members[0]
      : `z.union([${indented(members, depth)}])`;
  }
  if (schema.const !== undefined) return `z.literal(${JSON.stringify(schema.const)})`;

  const types = Array.isArray(schema.type) ? schema.type : schema.type ? [schema.type] : [];
  const nullable = types.includes("null");
  const kinds = types.filter((type) => type !== "null");
  if (kinds.length > 1) throw new Error(`unsupported type ${JSON.stringify(schema.type)}`);
  const base = kinds.length === 0 && !nullable ? "z.unknown()" : kinds.length === 0 ? "z.null()" : kind(schema, kinds[0], depth);
  return nullable && kinds.length > 0 ? `${base}.nullable()` : base;
}

function kind(schema: Schema, type: string, depth: number): string {
  switch (type) {
    case "string":
      if (!schema.enum) return "z.string()";
      return schema.enum.length === 1
        ? `z.literal(${quote(String(schema.enum[0]))})`
        : `z.enum([${schema.enum.map((member) => quote(String(member))).join(", ")}])`;
    case "integer":
      return "z.number().int()";
    case "number":
      return "z.number()";
    case "boolean":
      return "z.boolean()";
    case "array":
      return `z.array(${schema.items ? expression(schema.items, depth) : "z.unknown()"})`;
    case "object":
      return object(schema, depth);
    default:
      throw new Error(`unsupported type ${type}`);
  }
}

function object(schema: Schema, depth: number): string {
  const properties = Object.entries(schema.properties ?? {});
  if (properties.length === 0) {
    const values =
      typeof schema.additionalProperties === "object"
        ? expression(schema.additionalProperties, depth)
        : "z.unknown()";
    return `z.record(z.string(), ${values})`;
  }
  const required = new Set(schema.required ?? []);
  const fields = properties.map(([name, property]) => {
    const value = expression(property, depth + 1);
    const key = /^[A-Za-z_$][\w$]*$/.test(name) ? name : quote(name);
    return `${key}: ${required.has(name) ? value : `${value}.optional()`}`;
  });
  const constructor = schema.additionalProperties === false ? "z.strictObject" : "z.object";
  return `${constructor}({${indented(fields, depth)}})`;
}

function references(schema: unknown, found = new Set<string>()): Set<string> {
  if (Array.isArray(schema)) {
    for (const member of schema) references(member, found);
  } else if (typeof schema === "object" && schema !== null) {
    for (const [key, value] of Object.entries(schema)) {
      if (key === "$ref" && typeof value === "string") found.add(refName(value));
      else references(value, found);
    }
  }
  return found;
}

/** The component schemas, each after the ones it refers to. */
function inDependencyOrder(schemas: Record<string, Schema>): string[] {
  const ordered: string[] = [];
  const state = new Map<string, "visiting" | "done">();
  const visit = (name: string, trail: string[]) => {
    if (state.get(name) === "done") return;
    if (state.get(name) === "visiting") {
      throw new Error(`recursive schemas are not supported: ${[...trail, name].join(" -> ")}`);
    }
    const schema = schemas[name];
    if (!schema) throw new Error(`${name} is referenced but not defined`);
    state.set(name, "visiting");
    for (const dependency of [...references(schema)].sort()) visit(dependency, [...trail, name]);
    state.set(name, "done");
    ordered.push(name);
  };
  for (const name of Object.keys(schemas).sort()) visit(name, []);
  return ordered;
}

function jsonSchemaOf(content: Record<string, { schema?: Schema }> | undefined) {
  return content?.[JSON_CONTENT]?.schema;
}

/** The text of src/api/generated.ts for an OpenAPI document. */
export function generateApiModule(document: unknown): string {
  const { paths, components } = document as Document;
  const schemas = components?.schemas ?? {};
  const names = Object.keys(schemas).sort();
  for (const name of names) {
    if (!/^[A-Za-z_$][\w$]*$/.test(name)) throw new Error(`${name} is not a usable identifier`);
  }

  const out: string[] = [
    "// Generated by src/api/generate.ts from web/openapi.json: do not edit.",
    "// Run `make web-api` (or `pnpm generate` for a new web/openapi.json) to update it.",
    'import { z } from "zod";',
    "",
  ];

  for (const name of inDependencyOrder(schemas)) {
    out.push(`export const ${name}Schema = ${expression(schemas[name], 0)};`);
    out.push(`export type ${name} = z.infer<typeof ${name}Schema>;`);
    out.push("");
  }

  out.push("/** Every component schema by name. */");
  out.push(`export const schemas = {${indented(names.map((name) => `${name}: ${name}Schema`), 0)}};`);
  out.push("");

  const operations: string[] = [];
  for (const path of Object.keys(paths).sort()) {
    for (const method of HTTP_METHODS) {
      const operation = paths[path][method] as Operation | undefined;
      if (!operation) continue;
      const fields: string[] = [];
      const request = jsonSchemaOf(operation.requestBody?.content);
      if (request) fields.push(`request: ${expression(request, 2)}`);
      const responses: string[] = [];
      for (const status of Object.keys(operation.responses ?? {}).sort()) {
        const response = jsonSchemaOf(operation.responses?.[status]?.content);
        if (response) responses.push(`${quote(status)}: ${expression(response, 3)}`);
      }
      fields.push(`responses: {${indented(responses, 2)}}`);
      operations.push(`${quote(`${method.toUpperCase()} ${path}`)}: {${indented(fields, 1)}}`);
    }
  }
  out.push("/** The JSON request body and the JSON responses (by status) of every operation, by \"METHOD /path\". */");
  out.push(`export const operations = {${indented(operations, 0)}};`);
  out.push("");
  return out.join("\n");
}
