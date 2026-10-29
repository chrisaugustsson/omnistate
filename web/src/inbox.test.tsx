// @vitest-environment jsdom
// US-113: the Inbox, /w/<workspace id>/projects/<project id>/inbox. It lists
// the comments of GET .../home's `waiting.comments` (those whose latest reply
// is an agent's), grouped by scope and updated live. The app is rendered with
// its own router on a memory history, the browser's fetch is replaced by a
// fake of the API, and the browser's WebSocket by a fake socket that the test
// drives.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const CHECKOUT = "0192d7a0-4444-7000-8000-000000000001";
const WALLETS = "0192d7a0-4444-7000-8000-000000000002";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const INBOX_PATH = `${PROJECT_PATH}/inbox`;
const AT = "2026-10-21T09:00:00Z";
const HUMAN = { kind: "human", user_id: USER.id };

let counter = 0;
/** A UUIDv7-shaped ID that no other fixture has. */
function newId() {
  counter += 1;
  return `0192d7a0-3333-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

function doc(pillar: string, kind: string, title: string, feature: string | null = null) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: feature,
    kind,
    pillar,
    title,
    created_at: AT,
    drafts: [],
    claims: [],
  };
}

type Doc = ReturnType<typeof doc>;

function reply(author: Record<string, unknown>, text: string, at: string) {
  return { id: newId(), author, text, created_at: at };
}

function agent(role: string) {
  return { kind: "agent", run: "run-1", role, model: "test-model", harness: "test-harness" };
}

/** A comment of yours on `on` whose replies end with an agent's. */
function waiting(on: Doc, text: string, replies: ReturnType<typeof reply>[]) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: on.feature_id,
    doc_id: on.id,
    node: "p0000001",
    author: HUMAN,
    text,
    replies,
    orphaned: false,
    waiting_on_you: true,
    created_at: AT,
    snapshot: undefined as string | undefined,
  };
}

function feature(id: string, name: string) {
  return {
    feature: { id, project_id: PROJECT_ID, name, description: "", created_at: AT },
    docs: 1,
    open_changesets: 0,
    waiting: 1,
    last_activity: null,
  };
}

/**
 * Comments that wait in the app (agent replies at 11:00 and 14:00), in
 * Checkout (13:00) and in Wallets (12:30), listed in an order that is none of
 * the orders the Inbox shows.
 */
function seeded() {
  const docs = {
    returns: doc("spec", "html", "Returns PRD"),
    schema: doc("db", "html", "Orders schema"),
    pay: doc("ui", "html", "Pay screens notes", CHECKOUT),
    wallets: doc("research", "html", "Wallet interviews", WALLETS),
  };
  const comments = {
    wallets: waiting(docs.wallets, "Which wallets did they name?", [
      reply(agent("researcher"), "Apple Pay and Klarna.", "2026-10-21T12:30:00Z"),
    ]),
    schema: waiting(docs.schema, "Index the order date.", [
      reply(agent("architect"), "Added the index.", "2026-10-21T11:00:00Z"),
    ]),
    pay: waiting(docs.pay, "Drop the second step.", [
      reply(agent("designer"), "Merged it into one.", "2026-10-21T13:00:00Z"),
    ]),
    returns: waiting(docs.returns, "Say which week.", [
      reply(agent("writer"), "Seven days, I think.", "2026-10-21T10:00:00Z"),
      reply(HUMAN, "Check the policy.", "2026-10-21T10:30:00Z"),
      reply(agent("writer"), "Changed it to seven days, as the policy says.", "2026-10-21T14:00:00Z"),
    ]),
  };
  const home = {
    project_id: PROJECT_ID,
    app: { last_activity: null },
    features: [feature(CHECKOUT, "Checkout"), feature(WALLETS, "Wallets")],
    docs: Object.values(docs),
    changesets: [],
    threads: [],
    waiting: {
      decisions: [],
      changesets: [],
      questions: [],
      comments: [comments.wallets, comments.schema, comments.pay, comments.returns],
    },
  };
  return { home, docs, comments };
}

type Home = ReturnType<typeof seeded>["home"];

/** A fake of the API: the auth routes, the workspace's one project and its home, which the test may change. */
function fakeApi(home: Home) {
  const api = { home };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url, "http://localhost:3000").pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    switch (`${method} ${path}`) {
      case "GET /api/v1/auth/me":
        return json(200, { user: USER });
      case "GET /api/v1/auth/workspaces":
        return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
      case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
        return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
      case `GET /api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/home`:
        return json(200, api.home);
      default:
        return json(404, { error: "not_found" });
    }
  });
  vi.stubGlobal("fetch", fetch);
  return api;
}

/** A WebSocket the test drives: it records what the client sends. */
class FakeWebSocket extends EventTarget {
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
    if (this.readyState !== FakeWebSocket.OPEN) {
      throw new Error("InvalidStateError: the socket is not open");
    }
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

function renderAt(path: string) {
  const router = getRouter();
  router.update({
    ...router.options,
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  render(<RouterProvider router={router} />);
  return router;
}

/** The page itself, without the sidebar. */
async function page() {
  return screen.findByRole("main");
}

/** The Inbox's groups, each as its name and the comments of its items, in order. */
function groups(main: HTMLElement) {
  return within(main)
    .getAllByRole("region")
    .map((region) => ({
      name: (region.querySelector("h2")?.textContent ?? "").trim(),
      items: within(region)
        .getAllByRole("listitem")
        .map((item) => item.querySelector("a")?.textContent ?? ""),
    }));
}

/** The item of the Inbox that shows `text`. */
function itemShowing(main: HTMLElement, text: string) {
  const link = within(main).getByText(text).closest("a");
  expect(link, `the item showing ${text} is a link`).toBeTruthy();
  return link as HTMLAnchorElement;
}

describe("the Inbox", () => {
  beforeEach(() => {
    // An hour after the newest agent reply.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-21T15:00:00Z"));
    vi.stubGlobal("scrollTo", () => {});
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  test("the waiting comments come by scope, the newest agent reply first in the groups and in each group", async () => {
    fakeApi(seeded().home);
    renderAt(INBOX_PATH);
    const main = await page();
    await within(main).findByText("Say which week.");
    expect(groups(main)).toEqual([
      {
        name: "App",
        items: [expect.stringContaining("Say which week."), expect.stringContaining("Index the order date.")],
      },
      { name: "Checkout", items: [expect.stringContaining("Drop the second step.")] },
      { name: "Wallets", items: [expect.stringContaining("Which wallets did they name?")] },
    ]);
  });

  test("an item shows the doc and its pillar, your comment and the agent's latest reply with its role and age, and opens the comment", async () => {
    const { home, docs, comments } = seeded();
    fakeApi(home);
    renderAt(INBOX_PATH);
    const main = await page();
    await within(main).findByText("Say which week.");
    const item = itemShowing(main, "Say which week.");
    expect(within(item).getByText("Returns PRD")).toBeTruthy();
    expect(within(item).getByText("Spec")).toBeTruthy();
    expect(within(item).getByText("Changed it to seven days, as the policy says.")).toBeTruthy();
    expect(within(item).getByText("writer")).toBeTruthy();
    expect(within(item).getByText("1 h ago")).toBeTruthy();
    expect(item.textContent, "only the latest reply").not.toMatch(/Seven days, I think|Check the policy/);
    expect(item.getAttribute("href")).toBe(`${PROJECT_PATH}/docs/${docs.returns.id}?comment=${comments.returns.id}`);
  });

  test("a comment whose element was removed shows the element's snapshot, labelled Element removed", async () => {
    const { home, comments } = seeded();
    comments.pay.orphaned = true;
    comments.pay.snapshot = '<li data-omni-id="l0000001">Pay with Klarna</li>';
    fakeApi(home);
    renderAt(INBOX_PATH);
    const main = await page();
    await within(main).findByText("Drop the second step.");
    const item = itemShowing(main, "Drop the second step.");
    expect(within(item).getByText("Element removed")).toBeTruthy();
    expect(within(item).getByText("<li>")).toBeTruthy();
    expect(within(item).getByText("Pay with Klarna")).toBeTruthy();
    expect(within(itemShowing(main, "Say which week.")).queryByText("Element removed")).toBeNull();
  });

  test("a comment you replied to leaves the open Inbox as the reply's event comes in", async () => {
    const { home, comments } = seeded();
    const api = fakeApi(home);
    renderAt(INBOX_PATH);
    const main = await page();
    await within(main).findByText("Say which week.");

    await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
    const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
    socket.open();
    await waitFor(() =>
      expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })),
    );
    socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 10 });

    api.home = {
      ...api.home,
      waiting: { ...api.home.waiting, comments: [comments.wallets, comments.schema, comments.pay] },
    };
    socket.receive({
      type: "event",
      seq: 11,
      event: "comment.replied",
      project_id: PROJECT_ID,
      at: "2026-10-21T15:00:00Z",
      data: { comment_id: comments.returns.id, doc_id: comments.returns.doc_id },
    });
    await waitFor(() => expect(within(main).queryByText("Say which week.")).toBeNull());
    // The app's newest agent reply is now the one of 11:00, so the app comes last.
    expect(groups(main)).toEqual([
      { name: "Checkout", items: [expect.stringContaining("Drop the second step.")] },
      { name: "Wallets", items: [expect.stringContaining("Which wallets did they name?")] },
      { name: "App", items: [expect.stringContaining("Index the order date.")] },
    ]);
  });
});
