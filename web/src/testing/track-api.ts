// A fake of the API for the tests of the feature and track pages
// (track-page, track-scope, thread-view, object-decisions): one project with
// the parts Wision (app) and iot-service (service), its tracks, features, docs,
// threads and decisions as the test gives them, the workspace's members and
// the signed-in member. Writes are recorded and answered as the API would.
import { vi } from "vitest";

export const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
export const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
export const WISION = "0192d7a0-3333-7000-8000-000000000001";
export const IOT = "0192d7a0-3333-7000-8000-000000000002";
export const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
export const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
export const AT = "2026-11-10T08:00:00Z";

/** The signed-in member (alice), and two others. */
export const ALICE = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice.smith@example.com" };
export const BOB = "0192d7a0-1111-7000-8000-000000000002";
export const CARA = "0192d7a0-1111-7000-8000-000000000003";

export type Member = { membership_id: string; user_id: string; email: string; role: string };

export const member = (membershipId: string, userId: string, email: string, role = "member"): Member => ({
  membership_id: membershipId,
  user_id: userId,
  email,
  role,
});

/** The members most tests use: alice (the reader), bob and cara, all plain members. */
export const MEMBERS = [member("m-alice", ALICE.id, ALICE.email), member("m-bob", BOB, "bob@example.com"), member("m-cara", CARA, "cara.lee@example.com")];

export type Track = {
  id: string;
  project_id: string;
  part_id: string;
  state: string;
  name: string;
  description: string;
  owner_user_id: string | null;
  split_from_track_id: string | null;
  created_at: string;
};

export function track(id: string, name: string, fields: Partial<Track> = {}): Track {
  return {
    id,
    project_id: PROJECT_ID,
    part_id: WISION,
    state: "planning",
    name,
    description: "",
    owner_user_id: ALICE.id,
    split_from_track_id: null,
    created_at: AT,
    ...fields,
  };
}

export const featureBody = (id: string, name: string, fields: { part_id?: string; description?: string } = {}) => ({
  id,
  project_id: PROJECT_ID,
  part_id: fields.part_id ?? WISION,
  name,
  description: fields.description ?? "",
  created_at: AT,
});

export const docBody = (id: string, title: string, fields: { pillar: string; kind?: string; track_id?: string | null; feature_id?: string | null }) => ({
  id,
  project_id: PROJECT_ID,
  title,
  pillar: fields.pillar,
  kind: fields.kind ?? "html",
  track_id: fields.track_id ?? null,
  part_id: null,
  feature_id: fields.feature_id ?? null,
  created_at: AT,
  updated_at: AT,
});

type Json = Record<string, unknown>;

export type Fixture = {
  tracks: Track[];
  /** Story counts by track ID; none given is 0 of 0. */
  stories?: Record<string, { done: number; total: number }>;
  members?: Member[];
  me?: string;
  /** Features by the track that changes them. */
  trackFeatures?: Record<string, ReturnType<typeof featureBody>[]>;
  /** Tracks by the feature they change. */
  featureTracks?: Record<string, string[]>;
  features?: ReturnType<typeof featureBody>[];
  docs?: ReturnType<typeof docBody>[];
  threads?: Json[];
  /** Full threads by ID, as GET threads/{id} answers. */
  threadBodies?: Record<string, Json>;
  /** Stories of other tracks linked to each track's stories. */
  linked?: Record<string, Json[]>;
  /** Open questions of each track's behaviour tree. */
  questions?: Record<string, Json[]>;
  /** The nodes of each track's behaviour tree; none given is an empty tree. */
  tree?: Record<string, Json[]>;
  trackDecisions?: Record<string, Json[]>;
  featureDecisions?: Record<string, { decisions: Json[]; earlier: Json[] }>;
  /** The boards of each design doc's head, by doc ID. */
  boards?: Record<string, string[]>;
  /** Answers a write with this error instead. */
  refuse?: { status: number; error: string };
};

export type Call = { method: string; path: string; body: unknown };

/** Puts the fake in place of fetch; the returned object holds the calls made and the state the writes changed. */
export function fakeTrackApi(fixture: Fixture) {
  const api = { fixture, calls: [] as Call[] };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const summary = (one: Track) => ({
    track: one,
    docs: 0,
    waiting: 0,
    stories: { ...(fixture.stories?.[one.id] ?? { done: 0, total: 0 }), open: 0 },
    last_activity: null,
    part_name: one.part_id === IOT ? "iot-service" : "Wision",
    part_deleted: false,
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(typeof input === "string" ? input : input instanceof URL ? input.href : input.url, "http://localhost:3000");
      const path = url.pathname;
      const method = (init?.method ?? "GET").toUpperCase();
      const body = init?.body ? JSON.parse(String(init.body)) : undefined;
      api.calls.push({ method, path, body });
      const trackOf = (rest: string) => fixture.tracks.find((one) => path === `${API}/tracks/${one.id}${rest}`);

      if (method !== "GET") {
        if (fixture.refuse) return json(fixture.refuse.status, { error: fixture.refuse.error });
        if (path === `${API}/tracks`) {
          const created = track("0192d7a0-4444-7000-8000-0000000000ff", body.name, { description: body.description, part_id: body.part_id, owner_user_id: fixture.me ?? ALICE.id });
          fixture.tracks = [...fixture.tracks, created];
          return json(201, { track: created });
        }
        const linkedTo = path.match(/\/tracks\/([^/]+)\/features$/);
        if (linkedTo) {
          const feature = (fixture.features ?? []).find((one) => one.id === body.feature_id);
          return json(200, { feature });
        }
        const moved = trackOf("/state");
        if (moved) {
          const after = { ...moved, state: body.state };
          fixture.tracks = fixture.tracks.map((one) => (one.id === moved.id ? after : one));
          return json(200, { track: after });
        }
        const handed = trackOf("/owner");
        if (handed) {
          const to = (fixture.members ?? MEMBERS).find((one) => one.membership_id === body.membership_id);
          const after = { ...handed, owner_user_id: to?.user_id ?? null };
          fixture.tracks = fixture.tracks.map((one) => (one.id === handed.id ? after : one));
          return json(200, { track: after });
        }
        const recorded = trackOf("/decisions");
        if (recorded) {
          const created = {
            id: "0192d7a0-7777-7000-8000-0000000000ff",
            project_id: PROJECT_ID,
            track_id: recorded.id,
            about: body.feature_id ? "feature" : body.part_id ? "part" : body.about_project ? "project" : "track",
            feature_id: body.feature_id ?? null,
            feature_name: (fixture.trackFeatures?.[recorded.id] ?? []).find((one) => one.id === body.feature_id)?.name ?? null,
            part_id: body.part_id ?? null,
            part_name: body.part_id ? "Wision" : null,
            question: body.question,
            decision: body.decision,
            why: body.why,
            author: { kind: "human", user_id: fixture.me ?? ALICE.id },
            replaces_decision_id: null,
            replaced_by_decision_id: null,
            created_at: new Date().toISOString(),
          };
          fixture.trackDecisions = { ...fixture.trackDecisions, [recorded.id]: [created, ...(fixture.trackDecisions?.[recorded.id] ?? [])] };
          return json(201, { decision: created });
        }
        const described = (fixture.features ?? []).find((one) => method === "PATCH" && path === `${API}/parts/${one.part_id}/features/${one.id}`);
        if (described) {
          const after = { ...described, ...body, description: body.description?.trim() ?? described.description };
          fixture.features = (fixture.features ?? []).map((one) => (one.id === described.id ? after : one));
          return json(200, { feature: after });
        }
        const posted = path.match(/\/threads\/([^/]+)\/posts$/);
        if (posted) {
          return json(201, {
            post: {
              id: "0192d7a0-9999-7000-8000-0000000000ff",
              thread_id: posted[1],
              author: { kind: "human", user_id: fixture.me ?? ALICE.id },
              summary: body.summary,
              body: body.body,
              body_html: `<p>${body.body}</p>`,
              links: [],
              attachments: [],
              created_at: new Date().toISOString(),
            },
          });
        }
        return json(404, { error: "not_found" });
      }

      switch (path) {
        case "/api/v1/auth/me":
          return json(200, { user: ALICE });
        case "/api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `/api/v1/w/${WORKSPACE_ID}/whoami`:
          return json(200, { actor: { kind: "human", user_id: fixture.me ?? ALICE.id }, workspace_id: WORKSPACE_ID, workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `/api/v1/w/${WORKSPACE_ID}/members`:
          return json(200, { members: fixture.members ?? MEMBERS });
        case `/api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Monitoring platform", created_at: AT }] });
        case `${API}/parts`:
          return json(200, {
            parts: [
              { id: WISION, project_id: PROJECT_ID, name: "Wision", kind: "app", description: "", created_at: AT },
              { id: IOT, project_id: PROJECT_ID, name: "iot-service", kind: "service", description: "", created_at: "2026-11-10T09:00:00Z" },
            ],
          });
        case `${API}/home`:
          return json(200, {
            project_id: PROJECT_ID,
            app: { last_activity: null },
            tracks: fixture.tracks.map(summary),
            docs: fixture.docs ?? [],
            threads: fixture.threads ?? [],
            waiting: { comments: [] },
          });
      }
      for (const one of fixture.tracks) {
        const at = `${API}/tracks/${one.id}`;
        if (path === `${at}/features`) return json(200, { features: fixture.trackFeatures?.[one.id] ?? [] });
        if (path === `${at}/stories`) return json(200, { stories: [], linked: fixture.linked?.[one.id] ?? [] });
        if (path === `${at}/decisions`) return json(200, { decisions: fixture.trackDecisions?.[one.id] ?? [], earlier: [] });
        if (path === `${at}/behaviour/questions`) return json(200, { questions: fixture.questions?.[one.id] ?? [] });
        if (path === `${at}/behaviour`) return json(200, { nodes: fixture.tree?.[one.id] ?? [] });
      }
      for (const one of fixture.features ?? []) {
        const at = `${API}/parts/${one.part_id}/features/${one.id}`;
        if (path === at) return json(200, { feature: one });
        if (path === `${at}/tracks`) {
          const ids = fixture.featureTracks?.[one.id] ?? [];
          return json(200, { tracks: fixture.tracks.filter((candidate) => ids.includes(candidate.id)) });
        }
        if (path === `${at}/decisions`) return json(200, fixture.featureDecisions?.[one.id] ?? { decisions: [], earlier: [] });
        if (path === `${API}/parts/${one.part_id}/features`) return json(200, { features: (fixture.features ?? []).filter((candidate) => candidate.part_id === one.part_id) });
      }
      for (const doc of fixture.docs ?? []) {
        if (path === `${API}/docs/${doc.id}`) return json(200, { doc });
        if (path === `${API}/docs/${doc.id}/html`) {
          const boards = (fixture.boards?.[doc.id] ?? []).map((id) => ({ id, chunk: id }));
          return json(200, url.searchParams.get("view") === "outline" ? { doc_id: doc.id, html: "", outline: [], boards } : { doc_id: doc.id, html: "" });
        }
        if (path === `${API}/docs/${doc.id}/comments`) return json(200, { comments: [] });
      }
      const thread = Object.entries(fixture.threadBodies ?? {}).find(([id]) => path === `${API}/threads/${id}`);
      if (thread) return json(200, { thread: thread[1] });
      return json(404, { error: "not_found" });
    }),
  );
  return api;
}

/** A WebSocket the test drives: `receive` delivers a live event. */
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
    this.readyState = FakeWebSocket.CLOSED;
  }
  open() {
    this.readyState = FakeWebSocket.OPEN;
    this.fire("open", {});
  }
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
