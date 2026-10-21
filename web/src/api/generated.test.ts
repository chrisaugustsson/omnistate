// US-039: the web app's TypeScript types and Zod schemas are generated from the
// API's OpenAPI document. web/openapi.json is the web app's copy of the
// document (the API's own tests fail when it differs from what the API
// serves); src/api/generate.ts turns it into src/api/generated.ts, which is
// committed. This suite fails when generated.ts is not what the generator
// makes from openapi.json, so a stale copy fails the build.
//
// generated.ts exports, for every component schema `Name` of the document, a
// Zod schema `NameSchema` (and the type `Name`), all of them in `schemas` by
// name, and in `operations`, keyed by "METHOD /path", the Zod schemas of each
// operation's JSON request body (`request`) and JSON responses (`responses`,
// by status code). The messages of the live socket are the component schemas
// `LiveClientMessage` and `LiveServerMessage`.
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";

type Parser = {
  safeParse: (value: unknown) => { success: boolean };
};

type GeneratedModule = Record<string, unknown> & {
  schemas: Record<string, Parser>;
  operations: Record<
    string,
    { request?: Parser; responses: Record<string, Parser | undefined> }
  >;
};

type GeneratorModule = {
  generateApiModule: (document: unknown) => string | Promise<string>;
};

type OpenApiDocument = {
  paths: Record<string, Record<string, OpenApiOperation>>;
  components: { schemas: Record<string, unknown> };
};

type OpenApiOperation = {
  requestBody?: { content?: Record<string, { schema?: unknown }> };
  responses?: Record<string, { content?: Record<string, { schema?: unknown }> }>;
};

const HTTP_METHODS = ["get", "put", "post", "delete", "patch", "head", "options", "trace"];

const documentPath = fileURLToPath(new URL("../../openapi.json", import.meta.url));
const generatedPath = fileURLToPath(new URL("./generated.ts", import.meta.url));

function readDocument(): OpenApiDocument {
  expect(
    existsSync(documentPath),
    `the web app's copy of the OpenAPI document, ${documentPath}, exists`,
  ).toBe(true);
  return JSON.parse(readFileSync(documentPath, "utf8")) as OpenApiDocument;
}

/** Imports a module of the app, failing the test (not the suite) when it is missing. */
async function load<T>(specifier: string): Promise<T> {
  try {
    return (await import(/* @vite-ignore */ specifier)) as T;
  } catch (error) {
    expect.fail(`${specifier} cannot be imported: ${String(error)}`);
  }
}

const loadGenerated = () => load<GeneratedModule>("./generated");

function isSchema(value: unknown): value is Parser {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as Parser).safeParse === "function"
  );
}

function accepts(schema: unknown, value: unknown): boolean {
  expect(isSchema(schema), "a Zod schema").toBe(true);
  return (schema as Parser).safeParse(value).success;
}

const USER_ID = "0192d7a0-1111-7000-8000-000000000001";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000002";
const CHANGESET_ID = "0192d7a0-3333-7000-8000-000000000003";

describe("generated types and Zod schemas", () => {
  test("the committed module is what the generator makes from the committed OpenAPI document", async () => {
    const document = readDocument();
    const { generateApiModule } = await load<GeneratorModule>("./generate");
    expect(typeof generateApiModule).toBe("function");
    expect(existsSync(generatedPath), `${generatedPath} is committed`).toBe(true);
    const expected = await generateApiModule(document);
    expect(
      readFileSync(generatedPath, "utf8") === expected,
      "src/api/generated.ts is out of date: regenerate it from web/openapi.json",
    ).toBe(true);
  });

  test("every schema of the document has a Zod schema, by name and as NameSchema", async () => {
    const document = readDocument();
    const generated = await loadGenerated();
    const names = Object.keys(document.components.schemas);
    expect(names.length).toBeGreaterThan(0);
    for (const name of names) {
      expect(isSchema(generated.schemas?.[name]), `schemas.${name}`).toBe(true);
      expect(generated[`${name}Schema`], `${name}Schema`).toBe(generated.schemas[name]);
    }
  });

  test("every operation's JSON request body and JSON responses have a Zod schema", async () => {
    const document = readDocument();
    const generated = await loadGenerated();
    let checked = 0;
    for (const [path, item] of Object.entries(document.paths)) {
      for (const [method, operation] of Object.entries(item)) {
        if (!HTTP_METHODS.includes(method)) continue;
        const key = `${method.toUpperCase()} ${path}`;
        const described = generated.operations?.[key];
        expect(described, `operations["${key}"]`).toBeDefined();
        if (operation.requestBody?.content?.["application/json"]?.schema) {
          expect(isSchema(described.request), `the request of ${key}`).toBe(true);
          checked++;
        }
        for (const [status, response] of Object.entries(operation.responses ?? {})) {
          if (response.content?.["application/json"]?.schema) {
            expect(
              isSchema(described.responses?.[status]),
              `the ${status} response of ${key}`,
            ).toBe(true);
            checked++;
          }
        }
      }
    }
    expect(checked).toBeGreaterThan(0);
  });

  test("the login request and responses are checked as the API defines them", async () => {
    const generated = await loadGenerated();
    expect(accepts(generated.LoginRequestSchema, { email: "alice@example.com", password: "pw" })).toBe(true);
    expect(accepts(generated.LoginRequestSchema, { email: "alice@example.com" })).toBe(false);

    const login = generated.operations?.["POST /api/v1/auth/login"];
    expect(login, "operations of POST /api/v1/auth/login").toBeDefined();
    expect(accepts(login.request, { email: "alice@example.com", password: "pw" })).toBe(true);
    expect(
      accepts(login.responses["200"], { user: { id: USER_ID, email: "alice@example.com" } }),
    ).toBe(true);
    expect(accepts(login.responses["200"], { user: { id: USER_ID } })).toBe(false);
    expect(accepts(login.responses["401"], { error: "invalid_credentials" })).toBe(true);

    const workspaces = generated.operations?.["GET /api/v1/auth/workspaces"];
    expect(workspaces, "operations of GET /api/v1/auth/workspaces").toBeDefined();
    expect(
      accepts(workspaces.responses["200"], { workspaces: [{ id: USER_ID, name: "Acme" }] }),
    ).toBe(true);
    expect(accepts(workspaces.responses["200"], { workspaces: [{ id: USER_ID }] })).toBe(false);
  });

  test("the messages of the live socket have Zod schemas", async () => {
    const generated = await loadGenerated();
    const client = generated.LiveClientMessageSchema;
    expect(accepts(client, { type: "subscribe", project_id: PROJECT_ID })).toBe(true);
    expect(accepts(client, { type: "subscribe", project_id: PROJECT_ID, after: 3 })).toBe(true);
    expect(accepts(client, { type: "subscribe" })).toBe(false);

    const server = generated.LiveServerMessageSchema;
    expect(accepts(server, { type: "subscribed", project_id: PROJECT_ID, last_seq: 0 })).toBe(true);
    expect(accepts(server, { type: "error", error: "not_found" })).toBe(true);
    const event = {
      type: "event",
      seq: 4,
      event: "changeset.in_review",
      project_id: PROJECT_ID,
      at: "2026-10-21T10:00:00.000000Z",
      data: {
        changeset_id: CHANGESET_ID,
        from_status: "open",
        status: "in_review",
        actor: "agent",
      },
    };
    expect(accepts(server, event)).toBe(true);
    const withoutSeq: Record<string, unknown> = { ...event };
    delete withoutSeq.seq;
    expect(accepts(server, withoutSeq), "an event without its sequence number").toBe(false);
    expect(accepts(server, { type: "nonsense" })).toBe(false);
  });
});
