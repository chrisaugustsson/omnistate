// A fake of the API for the project's and the part's pages (Overview, Parts,
// Features, Docs, Tracks, Decisions), with a WebSocket the test drives and a
// render of the app at a URL. The tests describe the world they need; the fake
// answers from it and records every write.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { render } from "@testing-library/react";
import { vi } from "vitest";
import { getRouter } from "./router";

export const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
export const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
export const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
export const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
export const AT = "2026-11-10T08:00:00Z";
const ME = { id: "0192d7a0-1111-7000-8000-000000000001", email: "christopher.augustsson@example.com" };

let counter = 0;
/** A UUIDv7-shaped ID no other fixture has. */
export function newId(kind = "9999") {
  counter += 1;
  return `0192d7a0-${kind}-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

export type Member = { user_id: string; membership_id: string; role: string; email: string };
export function member(email: string, userId = newId("1111")): Member {
  return { user_id: userId, membership_id: newId("1112"), role: "member", email };
}
/** The signed-in member. */
export const me = member(ME.email, ME.id);

export function part(name: string, kind: "app" | "service", fields: { id?: string; description?: string; createdAt?: string } = {}) {
  return { id: fields.id ?? newId("3333"), project_id: PROJECT_ID, name, kind, description: fields.description ?? "", created_at: fields.createdAt ?? AT };
}
export type Part = ReturnType<typeof part>;

export function feature(partId: string, name: string, description = "") {
  return { id: newId("5555"), project_id: PROJECT_ID, part_id: partId, name, description, created_at: AT };
}
export type Feature = ReturnType<typeof feature>;

export function track(
  name: string,
  partId: string,
  state: string,
  fields: { owner?: string; done?: number; total?: number; lastActivity?: string | null; createdAt?: string; splitFrom?: string; partName?: string } = {},
) {
  const total = fields.total ?? 0;
  const done = fields.done ?? 0;
  return {
    track: {
      id: newId("4444"),
      project_id: PROJECT_ID,
      part_id: partId,
      state,
      name,
      description: "",
      owner_user_id: fields.owner ?? null,
      split_from_track_id: fields.splitFrom ?? null,
      created_at: fields.createdAt ?? AT,
    },
    docs: 0,
    waiting: 0,
    stories: { total, open: total - done, done },
    last_activity: fields.lastActivity === undefined ? null : fields.lastActivity,
    part_name: fields.partName ?? "",
    part_deleted: false,
  };
}
export type Track = ReturnType<typeof track>;

export function doc(title: string, pillar: string, fields: { part?: string; feature?: string; track?: string; updatedAt?: string; kind?: string } = {}) {
  return {
    id: newId("6666"),
    project_id: PROJECT_ID,
    track_id: fields.track ?? null,
    part_id: fields.part ?? null,
    feature_id: fields.feature ?? null,
    kind: fields.kind ?? "html",
    pillar,
    title,
    created_at: AT,
    updated_at: fields.updatedAt ?? AT,
    last_writer_role: "writer",
    waiting: 0,
  };
}
export type Doc = ReturnType<typeof doc>;

/** A decision recorded on `trackId`, about the project, a part or a feature, by a member or an agent. */
export function decision(
  text: string,
  trackId: string,
  about: { project: true } | { part: Part } | { feature: Feature; part: Part },
  fields: { by?: Member | string; at?: string; question?: string; why?: string; replaces?: string; replacedBy?: string } = {},
) {
  const author =
    typeof fields.by === "string" ? { kind: "agent", role: fields.by, run: "r1" } : { kind: "human", user_id: (fields.by ?? me).user_id };
  return {
    id: newId("7777"),
    project_id: PROJECT_ID,
    track_id: trackId,
    about: "project" in about ? "project" : "feature" in about ? "feature" : "part",
    part_id: "project" in about ? null : about.part.id,
    part_name: "project" in about ? null : about.part.name,
    feature_id: "feature" in about ? about.feature.id : null,
    feature_name: "feature" in about ? about.feature.name : null,
    question: fields.question ?? `Which way for “${text}”?`,
    decision: text,
    why: fields.why ?? "",
    author,
    replaces_decision_id: fields.replaces ?? null,
    replaced_by_decision_id: fields.replacedBy ?? null,
    created_at: fields.at ?? AT,
  };
}
export type Decision = ReturnType<typeof decision>;

export type World = {
  description?: string | null;
  members?: Member[];
  parts?: Part[];
  features?: Feature[];
  tracks?: Track[];
  /** The tracks that change each feature, by feature ID. */
  changes?: Record<string, string[]>;
  docs?: Doc[];
  decisions?: Decision[];
  /** How many open questions agents asked on each track, by track ID. */
  questions?: Record<string, number>;
};

export type Write = { method: string; path: string; body: unknown };

/** The decisions of a level and, as `earlier`, those of other levels they replace (as the API answers). */
function levelAnswer(all: Decision[], inLevel: (one: Decision) => boolean) {
  const decisions = all.filter(inLevel).sort((a, b) => b.created_at.localeCompare(a.created_at));
  const earlier: Decision[] = [];
  for (const one of decisions) {
    let next = all.find((other) => other.id === one.replaces_decision_id);
    while (next) {
      if (!inLevel(next) && !earlier.includes(next)) earlier.push(next);
      const at: Decision = next;
      next = all.find((other) => other.id === at.replaces_decision_id);
    }
  }
  return { decisions, earlier };
}

/** Stubs fetch with a fake of the API over `world`; the world may be changed by the test. */
export function fakeScopeApi(initial: World) {
  const world: Required<World> = {
    description: null,
    members: [me],
    parts: [],
    features: [],
    tracks: [],
    changes: {},
    docs: [],
    decisions: [],
    questions: {},
    ...initial,
  };
  const writes: Write[] = [];
  const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const W = `/api/v1/w/${WORKSPACE_ID}`;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const { pathname: path, searchParams } = new URL(url, "http://localhost:3000");
      const method = (init?.method ?? "GET").toUpperCase();
      const body = init?.body ? JSON.parse(String(init.body)) : null;
      if (method !== "GET") writes.push({ method, path, body });
      let at: RegExpMatchArray | null;

      if (method === "PATCH" && path === API) {
        if ("description" in body) world.description = body.description.trim() || null;
        return json(200, { project: { id: PROJECT_ID, name: "Monitoring platform", description: world.description, created_at: AT } });
      }
      if (method === "POST" && path === `${API}/parts`) {
        const created = part(body.name, body.kind, { description: body.description, createdAt: "2026-11-20T00:00:00Z" });
        world.parts = [...world.parts, created];
        return json(201, { part: created });
      }
      if ((at = path.match(new RegExp(`^${API}/parts/([^/]+)$`))) && method !== "GET") {
        const id = at[1];
        if (method === "DELETE") {
          world.parts = world.parts.filter((one) => one.id !== id);
          return new Response(null, { status: 204 });
        }
        world.parts = world.parts.map((one) => (one.id === id ? { ...one, name: body.name } : one));
        return json(200, { part: world.parts.find((one) => one.id === id) });
      }
      if ((at = path.match(new RegExp(`^${API}/parts/([^/]+)/features$`))) && method === "POST") {
        const created = feature(at[1], body.name, body.description);
        world.features = [...world.features, created];
        return json(201, { feature: created });
      }
      if ((at = path.match(new RegExp(`^${API}/parts/([^/]+)/features/([^/]+)$`))) && method !== "GET") {
        const id = at[2];
        if (method === "DELETE") {
          world.features = world.features.filter((one) => one.id !== id);
          return new Response(null, { status: 204 });
        }
        world.features = world.features.map((one) => (one.id === id ? { ...one, name: body.name } : one));
        return json(200, { feature: world.features.find((one) => one.id === id) });
      }
      if (method === "POST" && path === `${API}/tracks`) {
        const created = track(body.name, body.part_id, "planning");
        world.tracks = [...world.tracks, created];
        return json(201, { track: created.track });
      }
      if (method !== "GET") return json(404, { error: "not_found" });

      if ((at = path.match(new RegExp(`^${API}/parts/([^/]+)/features/([^/]+)/tracks$`)))) {
        const ids = world.changes[at[2]] ?? [];
        return json(200, { tracks: world.tracks.filter((one) => ids.includes(one.track.id)).map((one) => one.track) });
      }
      if ((at = path.match(new RegExp(`^${API}/parts/([^/]+)/features/([^/]+)/decisions$`)))) {
        const id = at[2];
        return json(200, levelAnswer(world.decisions, (one) => one.about === "feature" && one.feature_id === id));
      }
      if ((at = path.match(new RegExp(`^${API}/parts/([^/]+)/features$`)))) {
        const id = at[1];
        return json(200, { features: world.features.filter((one) => one.part_id === id) });
      }
      if ((at = path.match(new RegExp(`^${API}/parts/([^/]+)/decisions$`)))) {
        const id = at[1];
        return json(200, levelAnswer(world.decisions, (one) => one.about === "part" && one.part_id === id));
      }
      if ((at = path.match(new RegExp(`^${API}/tracks/([^/]+)/behaviour/questions$`)))) {
        const count = world.questions[at[1]] ?? 0;
        return json(200, {
          questions: Array.from({ length: count }, (_, place) => ({
            id: `q${place}`,
            node_id: "n1",
            node_path: "1",
            question: "Which one?",
            state: "open",
            answers: [],
            asked_by: { kind: "agent", role: "behaviour" },
            created_at: AT,
            version: 1,
          })),
        });
      }
      switch (path) {
        case "/api/v1/auth/me":
          return json(200, { user: { id: ME.id, email: ME.email } });
        case "/api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Wioniq" }] });
        case `${W}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Monitoring platform", description: world.description, created_at: AT }] });
        case `${W}/members`:
          return json(200, { members: world.members });
        case `${W}/whoami`:
          return json(200, { actor: { kind: "human", user_id: ME.id } });
        case `${API}/home`:
          return json(200, { project_id: PROJECT_ID, app: null, tracks: world.tracks, docs: world.docs, threads: [], waiting: { comments: [] } });
        case `${API}/parts`:
          return json(200, { parts: world.parts });
        case `${API}/decisions`:
          return searchParams.get("scope") === "project"
            ? json(200, levelAnswer(world.decisions, (one) => one.about === "project"))
            : json(400, { error: "invalid_request" });
        case `${API}/comments`:
          return json(200, { comments: [] });
        default:
          return json(404, { error: "not_found" });
      }
    }),
  );
  return { world, writes };
}

/** A WebSocket the test drives: it records what the client sends. */
export class FakeWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: FakeWebSocket[] = [];
  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;
  readyState = FakeWebSocket.CONNECTING;
  sent: unknown[] = [];
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: Event) => void) | null = null;
  onclose: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  constructor(readonly url: string) {
    super();
    FakeWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sent.push(JSON.parse(data));
  }

  close() {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.fire("close", { code: 1000, reason: "", wasClean: true });
  }

  /** The server accepted the connection. */
  open() {
    this.readyState = FakeWebSocket.OPEN;
    this.fire("open", {});
  }

  /** The server sent a JSON text frame. */
  receive(message: unknown) {
    this.fire("message", { data: JSON.stringify(message) });
  }

  private fire(type: string, fields: Record<string, unknown>) {
    const event = Object.assign(new Event(type), fields);
    const handler = (this as unknown as Record<string, unknown>)[`on${type}`];
    if (typeof handler === "function") handler.call(this, event);
    this.dispatchEvent(event);
  }
}

/** Renders the app at `path` on a memory history. */
export function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

/**
 * The text of `element` as a reader takes it in: each piece of text apart
 * (as the eye sees separate spans), without what is aria-hidden, its spaces
 * collapsed. What is only for screen readers counts.
 */
export function shownText(element: Element) {
  const copy = element.cloneNode(true) as HTMLElement;
  for (const hidden of Array.from(copy.querySelectorAll("[aria-hidden]"))) hidden.remove();
  const walker = document.createTreeWalker(copy, NodeFilter.SHOW_TEXT);
  const pieces: string[] = [];
  for (let node = walker.nextNode(); node; node = walker.nextNode()) pieces.push(node.textContent ?? "");
  return pieces.join(" ").replace(/\s+/g, " ").replace(/ ([,.”])/g, "$1").replace(/“ /g, "“").trim();
}
