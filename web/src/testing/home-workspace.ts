import { vi } from "vitest";

// The workspace that Home's tests (web/src/home.test.tsx, web/src/inbox.test.tsx)
// run against, and a fake of the API that serves it. Two projects: Storefront
// (parts Web, an app, and payments-api, a service) and Warehouse (part
// scanner). Alice, the signed-in member, owns Checkout, Returns and Apple Pay
// in Storefront and Pick lists in Warehouse; Bob owns Wallets. Five comments
// wait (their latest reply is an agent's): two on Checkout, one on Pick lists,
// one on Bob's Wallets and one on a Storefront doc of no track.

export const WS = "0192d7a0-0000-7000-8000-00000000000a";
export const ALICE = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice.doe@example.com" };
export const BOB = { id: "0192d7a0-1111-7000-8000-000000000002", email: "bob.smith@example.com" };
export const STOREFRONT = "0192d7a0-2222-7000-8000-000000000001";
export const WAREHOUSE = "0192d7a0-2222-7000-8000-000000000002";
export const WEB = "0192d7a0-3333-7000-8000-000000000001";
export const PAYMENTS_API = "0192d7a0-3333-7000-8000-000000000002";
export const SCANNER = "0192d7a0-3333-7000-8000-000000000003";
export const CHECKOUT = "0192d7a0-4444-7000-8000-000000000001";
export const WALLETS = "0192d7a0-4444-7000-8000-000000000002";
export const RETURNS = "0192d7a0-4444-7000-8000-000000000003";
export const APPLE_PAY = "0192d7a0-4444-7000-8000-000000000004";
export const PICK_LISTS = "0192d7a0-4444-7000-8000-000000000005";
export const PAY_SCREENS = "0192d7a0-5555-7000-8000-000000000001";
export const WALLET_NOTES = "0192d7a0-5555-7000-8000-000000000002";
export const STOREFRONT_PRD = "0192d7a0-5555-7000-8000-000000000003";
export const PICK_SPEC = "0192d7a0-5555-7000-8000-000000000004";
/** The waiting comments: two on Checkout, one on Pick lists, one on Wallets, one on the Storefront PRD. */
export const MERGED = "0192d7a0-8888-7000-8000-000000000001";
export const MOVED = "0192d7a0-8888-7000-8000-000000000002";
export const SCANNER_FLOW = "0192d7a0-8888-7000-8000-000000000003";
export const KLARNA = "0192d7a0-8888-7000-8000-000000000004";
export const SEVEN_DAYS = "0192d7a0-8888-7000-8000-000000000005";
export const API = `/api/v1/w/${WS}`;
const AT = "2026-10-01T09:00:00Z";

/** 2026-10-21 at `hh:mm` local time, as an ISO string. */
export function today(hhmm: string): string {
  const [hours, minutes] = hhmm.split(":").map(Number);
  return new Date(2026, 9, 21, hours, minutes).toISOString();
}

const part = (id: string, project: string, name: string, kind: string, created: string) => ({
  id,
  project_id: project,
  name,
  kind,
  description: "",
  created_at: created,
});

function track(
  id: string,
  project: string,
  partId: string,
  partName: string,
  name: string,
  state: string,
  owner: string,
  stories: { total: number; done: number },
  lastActivity: string,
) {
  return {
    track: { id, project_id: project, part_id: partId, name, description: "", state, owner_user_id: owner, created_at: AT },
    part_name: partName,
    docs: 1,
    waiting: 0,
    stories: { total: stories.total, done: stories.done, open: stories.total - stories.done },
    last_activity: lastActivity,
  };
}

const doc = (id: string, project: string, trackId: string | null, pillar: string, title: string) => ({
  id,
  project_id: project,
  track_id: trackId,
  kind: "html",
  pillar,
  title,
  created_at: AT,
  updated_at: AT,
});

export const agent = (role: string) => ({ kind: "agent", role, run: "run-1", model: "test-model", harness: "test-harness" });

/** Alice's comment `text` on a doc, whose replies end with `replies`' last. */
export function comment(
  id: string,
  project: string,
  docId: string,
  trackId: string | null,
  text: string,
  replies: { author: Record<string, unknown>; text: string; at: string }[],
) {
  return {
    id,
    project_id: project,
    track_id: trackId,
    doc_id: docId,
    node: "p0000001",
    author: { kind: "human", user_id: ALICE.id },
    text,
    replies: replies.map((reply, at) => ({ id: `${id.slice(0, -2)}9${at}`, author: reply.author, text: reply.text, created_at: reply.at })),
    orphaned: false as boolean,
    snapshot: undefined as string | undefined,
    waiting_on_you: true,
    created_at: AT,
  };
}

export function storefrontHome() {
  return {
    project_id: STOREFRONT,
    tracks: [
      track(CHECKOUT, STOREFRONT, WEB, "Web", "Checkout", "planning", ALICE.id, { total: 4, done: 1 }, today("10:00")),
      track(WALLETS, STOREFRONT, PAYMENTS_API, "payments-api", "Wallets", "building", BOB.id, { total: 3, done: 0 }, today("09:00")),
      track(RETURNS, STOREFRONT, WEB, "Web", "Returns", "done", ALICE.id, { total: 3, done: 3 }, today("08:00")),
      track(APPLE_PAY, STOREFRONT, PAYMENTS_API, "payments-api", "Apple Pay", "dropped", ALICE.id, { total: 0, done: 0 }, today("07:00")),
    ],
    docs: [
      doc(PAY_SCREENS, STOREFRONT, CHECKOUT, "ui", "Pay screens"),
      doc(WALLET_NOTES, STOREFRONT, WALLETS, "research", "Wallet notes"),
      doc(STOREFRONT_PRD, STOREFRONT, null, "spec", "Storefront PRD"),
    ],
    threads: [],
    waiting: {
      comments: [
        comment(MOVED, STOREFRONT, PAY_SCREENS, CHECKOUT, "Move the pay button up.", [
          { author: agent("designer"), text: "Moved the button above the fold.", at: today("11:00") },
        ]),
        comment(KLARNA, STOREFRONT, WALLET_NOTES, WALLETS, "Which wallets did they name?", [
          { author: agent("researcher"), text: "Apple Pay and Klarna.", at: today("14:30") },
        ]),
        comment(SEVEN_DAYS, STOREFRONT, STOREFRONT_PRD, null, "Say which week.", [
          { author: agent("writer"), text: "Seven days, as the policy says.", at: today("12:00") },
        ]),
        comment(MERGED, STOREFRONT, PAY_SCREENS, CHECKOUT, "Drop the second step.", [
          { author: agent("designer"), text: "I would keep it.", at: today("10:00") },
          { author: { kind: "human", user_id: ALICE.id }, text: "Drop it anyway.", at: today("12:00") },
          { author: agent("designer"), text: "Merged the two steps into one.", at: today("13:00") },
        ]),
      ],
    },
  };
}

export function warehouseHome() {
  return {
    project_id: WAREHOUSE,
    tracks: [track(PICK_LISTS, WAREHOUSE, SCANNER, "scanner", "Pick lists", "building", ALICE.id, { total: 2, done: 0 }, today("09:30"))],
    docs: [doc(PICK_SPEC, WAREHOUSE, PICK_LISTS, "spec", "Pick list spec")],
    threads: [],
    waiting: {
      comments: [
        comment(SCANNER_FLOW, WAREHOUSE, PICK_SPEC, PICK_LISTS, "How does the scanner start?", [
          { author: agent("writer"), text: "Added the scanner flow.", at: today("14:00") },
        ]),
      ],
    },
  };
}

type Home = ReturnType<typeof storefrontHome> | ReturnType<typeof warehouseHome>;

const question = (asker: Record<string, unknown>, n: number) => ({
  id: `0192d7a0-9999-7000-8000-00000000000${n}`,
  node_id: "n1",
  node_path: "checkout",
  question: `Question ${n}?`,
  asked_by: asker,
  answers: [],
  state: "open",
  version: 1,
  created_at: AT,
});

const story = (key: string, status: string, blockers: { key: string; status: string; track: string }[] = []) => ({
  id: `0192d7a0-aaaa-7000-8000-0000000000${key.slice(3).padStart(2, "0")}`,
  key,
  number: Number(key.slice(3)),
  title: `Story ${key}`,
  text: "",
  topic: "",
  priority: 2,
  project_id: STOREFRONT,
  track_id: CHECKOUT,
  criteria: [],
  decisions: [],
  files_changed: [],
  files_planned: [],
  findings: [],
  links: [],
  version: 1,
  created_at: AT,
  updated_at: AT,
  status,
  blocked: false,
  blocked_by: [],
  blockers: blockers.map((blocker, n) => ({
    id: `0192d7a0-bbbb-7000-8000-00000000000${n}`,
    key: blocker.key,
    status: blocker.status,
    track_id: WALLETS,
    track_name: blocker.track,
    blocker_dropped: false,
  })),
});

/**
 * A fake of the API. `me` is the signed-in member; `api.homes` may be changed
 * by the test, and `api.replies` records each reply posted. Checkout has three
 * open questions an agent asked and one remark Alice left, and its US-2 waits
 * on Wallets' US-7 (US-1, done, waited on a story that is not done either).
 */
export function fakeApi(me = ALICE) {
  const api = {
    homes: { [STOREFRONT]: storefrontHome(), [WAREHOUSE]: warehouseHome() } as Record<string, Home>,
    replies: [] as { path: string; text: string }[],
  };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const fixed: Record<string, unknown> = {
    "/api/v1/auth/me": { user: me },
    "/api/v1/auth/workspaces": { workspaces: [{ id: WS, name: "Acme" }] },
    [`${API}/whoami`]: { actor: { kind: "human", user_id: me.id }, workspace_id: WS },
    [`${API}/members`]: {
      members: [
        { membership_id: "m1", user_id: ALICE.id, email: ALICE.email, role: "admin" },
        { membership_id: "m2", user_id: BOB.id, email: BOB.email, role: "member" },
      ],
    },
    [`${API}/projects`]: {
      projects: [
        { id: STOREFRONT, name: "Storefront", created_at: AT },
        { id: WAREHOUSE, name: "Warehouse", created_at: AT },
      ],
    },
    [`${API}/projects/${STOREFRONT}/parts`]: {
      parts: [part(WEB, STOREFRONT, "Web", "app", "2026-09-01T00:00:00Z"), part(PAYMENTS_API, STOREFRONT, "payments-api", "service", "2026-09-02T00:00:00Z")],
    },
    [`${API}/projects/${WAREHOUSE}/parts`]: { parts: [part(SCANNER, WAREHOUSE, "scanner", "app", AT)] },
    [`${API}/projects/${STOREFRONT}/tracks/${CHECKOUT}/behaviour/questions`]: {
      questions: [question(agent("behaviour"), 1), question(agent("behaviour"), 2), question({ kind: "human", user_id: ALICE.id }, 3), question(agent("behaviour"), 4)],
    },
    [`${API}/projects/${STOREFRONT}/tracks/${CHECKOUT}/stories`]: {
      stories: [
        story("US-1", "done", [{ key: "US-6", status: "review", track: "Wallets" }]),
        story("US-2", "backlog", [{ key: "US-7", status: "implementing", track: "Wallets" }]),
      ],
      linked: [],
    },
  };
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url, "http://localhost:3000").pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    const home = path.match(/^\/api\/v1\/w\/[^/]+\/projects\/([^/]+)\/home$/);
    if (method === "GET" && home && api.homes[home[1]]) return json(200, api.homes[home[1]]);
    const replied = path.match(/\/projects\/([^/]+)\/comments\/([^/]+)\/replies$/);
    if (method === "POST" && replied) {
      const text = (JSON.parse(String(init?.body)) as { text: string }).text;
      api.replies.push({ path, text });
      const project = api.homes[replied[1]];
      const answered = project.waiting.comments.find((candidate) => candidate.id === replied[2])!;
      const reply = { id: "0192d7a0-cccc-7000-8000-000000000001", author: { kind: "human", user_id: me.id }, text, created_at: today("15:00") };
      // The comment no longer waits: its latest reply is now the human's.
      project.waiting.comments = project.waiting.comments.filter((candidate) => candidate.id !== replied[2]);
      return json(201, { comment: { ...answered, replies: [...answered.replies, reply] } });
    }
    if (method === "GET" && path in fixed) return json(200, fixed[path]);
    if (method === "GET" && path.endsWith("/behaviour/questions")) return json(200, { questions: [] });
    if (method === "GET" && path.endsWith("/stories")) return json(200, { stories: [], linked: [] });
    return json(404, { error: "not_found" });
  });
  vi.stubGlobal("fetch", fetch);
  return api;
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
    if (this.readyState !== FakeWebSocket.OPEN) throw new Error("InvalidStateError: the socket is not open");
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
