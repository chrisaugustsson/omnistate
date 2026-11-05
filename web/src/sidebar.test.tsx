// @vitest-environment jsdom
// US-123: the project sidebar (the navigation named "Project" below the
// switcher) and the visit history in localStorage, omni:visits:<project id>.
// US-124: the sidebar's Recent and scope tree.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const OTHER_WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000b";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const OTHER_PROJECT_ID = "0192d7a0-2222-7000-8000-000000000002";
const CHECKOUT = "0192d7a0-4444-7000-8000-000000000001";
const WALLETS = "0192d7a0-4444-7000-8000-000000000002";
const SEARCH = "0192d7a0-4444-7000-8000-000000000003";
const APP_DOC = "0192d7a0-5555-7000-8000-000000000001";
const CHECKOUT_DOC = "0192d7a0-5555-7000-8000-000000000002";
const CHECKOUT_THREAD = "0192d7a0-6666-7000-8000-000000000001";
const SAY_WEEK = "0192d7a0-7777-7000-8000-000000000001";
const INDEX_DATE = "0192d7a0-7777-7000-8000-000000000002";
const RETURNS = "0192d7a0-4444-7000-8000-000000000004";
const GIFT_CARDS = "0192d7a0-4444-7000-8000-000000000005";
const LOYALTY = "0192d7a0-4444-7000-8000-000000000006";
const GONE = "0192d7a0-4444-7000-8000-000000000009";
const MISSING = "0192d7a0-9999-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const WORKSPACE_PATH = `/w/${WORKSPACE_ID}`;
const PROJECT_PATH = `${WORKSPACE_PATH}/projects/${PROJECT_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const VISITS_KEY = `omni:visits:${PROJECT_ID}`;
const AT = "2026-10-21T09:00:00Z";
const AGENT = { kind: "agent", run: "run-1", role: "writer", model: "test-model", harness: "test-harness" };
const HUMAN = { kind: "human", user_id: USER.id };

function feature(id: string, name: string, stories = { total: 0, open: 0, done: 0 }) {
  return {
    feature: { id, project_id: PROJECT_ID, name, description: "", created_at: AT },
    docs: 0,
    waiting: 0,
    stories,
    last_activity: null,
  };
}

function doc(id: string, featureId: string | null, title: string, fields: { pillar?: string; kind?: string; waiting?: number } = {}) {
  return {
    id,
    project_id: PROJECT_ID,
    feature_id: featureId,
    kind: fields.kind ?? "html",
    pillar: fields.pillar ?? "spec",
    waiting: fields.waiting ?? 0,
    title,
    created_at: AT,
    drafts: [],
    claims: [],
  };
}

function thread(id: string, featureId: string | null, question: string) {
  return {
    id,
    project_id: PROJECT_ID,
    feature_id: featureId,
    question,
    sealed: false,
    conclusion: null,
    run_tag: null,
    created_at: AT,
    post_count: 0,
    last_post_at: null,
  };
}

/** A comment of yours on the app's doc whose latest reply is an agent's. */
function waiting(id: string, text: string) {
  return {
    id,
    project_id: PROJECT_ID,
    feature_id: null,
    doc_id: APP_DOC,
    node: "p0000001",
    author: HUMAN,
    text,
    replies: [{ id: "0192d7a0-8888-7000-8000-000000000001", author: AGENT, text: "Done.", created_at: AT }],
    orphaned: false,
    waiting_on_you: true,
    created_at: AT,
  };
}

const docs = [doc(APP_DOC, null, "Returns PRD"), doc(CHECKOUT_DOC, CHECKOUT, "Pay in one step")];
const threads = [thread(CHECKOUT_THREAD, CHECKOUT, "Which wallets?")];

function home(
  fields: {
    features?: ReturnType<typeof feature>[];
    waiting?: ReturnType<typeof waiting>[];
    docs?: ReturnType<typeof doc>[];
    threads?: ReturnType<typeof thread>[];
  } = {},
) {
  return {
    project_id: PROJECT_ID,
    app: { last_activity: null },
    features: fields.features ?? [feature(CHECKOUT, "Checkout")],
    docs: fields.docs ?? docs,
    threads: fields.threads ?? threads,
    waiting: { comments: fields.waiting ?? [] },
  };
}

type Home = ReturnType<typeof home>;
type Workspace = { id: string; name: string };

const ACME = { id: WORKSPACE_ID, name: "Acme" };

/** A fake of the API: the auth routes, the account's workspaces, the workspace's two projects, the home, and the docs and threads of `home`. */
function fakeApi(fields: { home?: Home; workspaces?: Workspace[] } = {}) {
  const api = { home: fields.home ?? home(), projectName: "Storefront", projectsRequests: 0 };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url, "http://localhost:3000").pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    if (method !== "GET") return json(404, { error: "not_found" });
    switch (path) {
      case "/api/v1/auth/me":
        return json(200, { user: USER });
      case "/api/v1/auth/workspaces":
        return json(200, { workspaces: fields.workspaces ?? [ACME] });
      case `/api/v1/w/${WORKSPACE_ID}/projects`:
        api.projectsRequests += 1;
        return json(200, {
          projects: [
            { id: PROJECT_ID, name: api.projectName, created_at: AT },
            { id: OTHER_PROJECT_ID, name: "Backoffice", created_at: AT },
          ],
        });
      case `/api/v1/w/${OTHER_WORKSPACE_ID}/projects`:
        return json(200, { projects: [] });
      case `${API}/home`:
        return json(200, api.home);
      case `${API}/comments`:
        return json(200, { project_id: PROJECT_ID, comments: [] });
      default:
        break;
    }
    const shownDoc = api.home.docs.find((candidate) => path.startsWith(`${API}/docs/${candidate.id}`));
    if (shownDoc && path === `${API}/docs/${shownDoc.id}`) return json(200, { doc: shownDoc });
    if (shownDoc && path === `${API}/docs/${shownDoc.id}/html`) {
      return json(200, { doc_id: shownDoc.id, html: `<p data-omni-id="p0000001">${shownDoc.title}</p>` });
    }
    const shownThread = api.home.threads.find((candidate) => path === `${API}/threads/${candidate.id}`);
    if (shownThread) return json(200, { thread: { ...shownThread, posts: [] } });
    return json(404, { error: "not_found" });
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
    if (this.readyState !== FakeWebSocket.OPEN) throw new Error("InvalidStateError: the socket is not open");
    this.sent.push(JSON.parse(data));
  }

  close() {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.fire("close", { code: 1000, reason: "", wasClean: true });
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

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

/** The project's places in the sidebar. */
async function projectNav() {
  return screen.findByRole("navigation", { name: "Project" });
}

/** The fixed places at the top of the project's sidebar, without Recent and the scope tree. */
function places(nav: HTMLElement) {
  return within(nav).getByRole("link", { name: /^Inbox/ }).parentElement as HTMLElement;
}

/** Each link of `container` as its text and href, in order. */
function links(container: HTMLElement) {
  return within(container)
    .queryAllByRole("link")
    .map((link) => ({ text: (link.textContent ?? "").trim(), href: link.getAttribute("href") }));
}

/** The text of `element` without its decoration (what is aria-hidden). */
function shownText(element: HTMLElement) {
  const copy = element.cloneNode(true) as HTMLElement;
  for (const hidden of Array.from(copy.querySelectorAll("[aria-hidden]"))) hidden.remove();
  return (copy.textContent ?? "").trim();
}

/** The switcher's menu, opened. */
async function openMenu() {
  fireEvent.click(await screen.findByRole("button", { name: "Switch project" }));
  return screen.findByRole("menu");
}

function menuItems(menu: HTMLElement) {
  return within(menu)
    .getAllByRole("menuitem")
    .map((item) => ({ text: (item.textContent ?? "").trim(), href: item.getAttribute("href") }));
}

function storedVisits() {
  return JSON.parse(localStorage.getItem(VISITS_KEY) ?? "[]") as unknown[];
}

/** Opens `path` at the time `at`, and waits until `ready` holds. */
async function visit(router: ReturnType<typeof renderAt>, path: string, at: string, ready: () => void) {
  vi.setSystemTime(new Date(at));
  await router.navigate({ href: path });
  await waitFor(ready);
  await settled();
}

/** Lets the effects of what was rendered run. */
async function settled() {
  await act(async () => {});
}

/** The page shows `text`. */
const shows = (text: string | RegExp) => () =>
  expect(within(screen.getByRole("main")).queryAllByText(text)).not.toEqual([]);

/** The doc has loaded: its head is in the frame. */
const docShown = () => expect(screen.getByRole("main").querySelector("iframe")).toBeTruthy();

/** The thread has loaded: its question is the page's heading. */
const threadShown = (question: string) => () =>
  expect(within(screen.getByRole("main")).getByRole("heading", { level: 1, name: question })).toBeTruthy();

describe("the project sidebar", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
    localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  describe("its places", () => {
    test("Inbox with what waits, App, Features with their number, and every scope's Threads, and no feature", async () => {
      fakeApi({
        home: home({
          features: [feature(CHECKOUT, "Checkout"), feature(WALLETS, "Wallets"), feature(SEARCH, "Search")],
          waiting: [waiting(SAY_WEEK, "Say which week."), waiting(INDEX_DATE, "Index the order date.")],
        }),
      });
      renderAt(`${PROJECT_PATH}/app`);
      const nav = await projectNav();
      await waitFor(() => expect(links(nav)[2]?.text).toBe("Features3"));
      expect(links(places(nav))).toEqual([
        { text: "Inbox2", href: `${PROJECT_PATH}/inbox` },
        { text: "App", href: `${PROJECT_PATH}/app` },
        { text: "Features3", href: `${PROJECT_PATH}/features` },
        { text: "Threads", href: `${PROJECT_PATH}/threads` },
      ]);
    });

    test("with nothing waiting and no features, the Inbox has no count and Features says 0", async () => {
      fakeApi({ home: home({ features: [], waiting: [] }) });
      renderAt(`${PROJECT_PATH}/app`);
      const nav = await projectNav();
      await waitFor(() => expect(links(nav)[2]?.text).toBe("Features0"));
      expect(links(places(nav)).map((link) => link.text)).toEqual(["Inbox", "App", "Features0", "Threads"]);
    });

    test("the Inbox count follows the comments as their events come in", async () => {
      const api = fakeApi({ home: home({ waiting: [waiting(SAY_WEEK, "Say which week."), waiting(INDEX_DATE, "Index the order date.")] }) });
      renderAt(`${PROJECT_PATH}/features`);
      const nav = await projectNav();
      await waitFor(() => expect(links(nav)[0]?.text).toBe("Inbox2"));

      await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
      const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
      socket.open();
      await waitFor(() =>
        expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })),
      );
      socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 10 });
      api.home = home({ waiting: [waiting(INDEX_DATE, "Index the order date.")] });
      socket.receive({
        type: "event",
        seq: 11,
        event: "comment.replied",
        project_id: PROJECT_ID,
        at: "2026-10-21T15:00:00Z",
        data: { comment_id: SAY_WEEK, doc_id: APP_DOC },
      });
      await waitFor(() => expect(links(nav)[0]?.text).toBe("Inbox1"));
    });
  });

  describe("a rename", () => {
    async function openSocket() {
      await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
      const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
      socket.open();
      await waitFor(() =>
        expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })),
      );
      socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 10 });
      return socket;
    }

    function renamed(projectId: string, seq: number) {
      return {
        type: "event",
        event: "project.renamed",
        project_id: projectId,
        seq,
        at: AT,
        data: { project_id: projectId, name: "Shop v2" },
      };
    }

    test("an open App page shows the new name in its title, the switcher and its list, one reload per consumer", async () => {
      const api = fakeApi();
      renderAt(`${PROJECT_PATH}/app`);
      const button = await screen.findByRole("button", { name: "Switch project" });
      await waitFor(() => expect(shownText(button)).toBe("AcmeStorefront"));
      const socket = await openSocket();
      await settled();
      const before = api.projectsRequests;

      api.projectName = "Shop v2";
      socket.receive(renamed(PROJECT_ID, 11));
      await waitFor(() => expect(shownText(button)).toBe("AcmeShop v2"));
      await waitFor(() =>
        expect(within(screen.getByRole("main")).getByRole("heading", { level: 1, name: "Shop v2" })).toBeTruthy(),
      );
      expect(api.projectsRequests).toBe(before + 2);
      expect(menuItems(await openMenu()).map((item) => item.text)).toContain("Shop v2");
    });

    test("another project's event reloads no projects", async () => {
      const api = fakeApi();
      renderAt(`${PROJECT_PATH}/app`);
      await waitFor(() => expect(shownText(screen.getByRole("button", { name: "Switch project" }))).toBe("AcmeStorefront"));
      const socket = await openSocket();
      await settled();
      const before = api.projectsRequests;

      socket.receive(renamed(OTHER_PROJECT_ID, 11));
      await settled();
      expect(api.projectsRequests).toBe(before);
    });

    test("the workspace home subscribes to no project", async () => {
      fakeApi();
      renderAt(WORKSPACE_PATH);
      await within(await screen.findByRole("main")).findByText("Storefront");
      await settled();
      expect(FakeWebSocket.instances.flatMap((socket) => socket.sent)).toEqual([]);
    });
  });

  describe("the switcher", () => {
    test("names the workspace above the project, and lists the projects and All projects, no workspaces with one", async () => {
      fakeApi();
      renderAt(`${PROJECT_PATH}/app`);
      const button = await screen.findByRole("button", { name: "Switch project" });
      await waitFor(() => expect(shownText(button)).toBe("AcmeStorefront"));
      const menu = await openMenu();
      await waitFor(() =>
        expect(menuItems(menu)).toEqual([
          { text: "Storefront", href: PROJECT_PATH },
          { text: "Backoffice", href: `${WORKSPACE_PATH}/projects/${OTHER_PROJECT_ID}` },
          { text: "All projects", href: WORKSPACE_PATH },
        ]),
      );
    });

    test("with two workspaces it lists them, and choosing the other opens its All projects page", async () => {
      fakeApi({ workspaces: [ACME, { id: OTHER_WORKSPACE_ID, name: "Tirebud" }] });
      const router = renderAt(`${PROJECT_PATH}/app`);
      const menu = await openMenu();
      await waitFor(() => expect(menuItems(menu)).toHaveLength(5));
      expect(menuItems(menu).slice(3)).toEqual([
        { text: "Acme", href: WORKSPACE_PATH },
        { text: "Tirebud", href: `/w/${OTHER_WORKSPACE_ID}` },
      ]);

      fireEvent.click(within(menu).getByRole("menuitem", { name: "Tirebud" }));
      await waitFor(() => expect(router.state.location.pathname).toBe(`/w/${OTHER_WORKSPACE_ID}`));
      await waitFor(() =>
        expect(within(screen.getByRole("main")).getByRole("heading", { level: 1, name: "Tirebud" })).toBeTruthy(),
      );
    });

    test("outside a project the sidebar holds the header with the same menu and the footer only", async () => {
      fakeApi();
      renderAt(WORKSPACE_PATH);
      const button = await screen.findByRole("button", { name: "Switch project" });
      await within(await screen.findByRole("main")).findByText("Storefront");
      expect(shownText(button)).toBe("Acme");
      expect(links(await projectNav())).toEqual([]);
      expect(screen.getByRole("link", { name: "API keys" }).getAttribute("href")).toBe(`${WORKSPACE_PATH}/api-keys`);
      expect(screen.getByRole("button", { name: "Log out" })).toBeTruthy();

      const menu = await openMenu();
      await waitFor(() =>
        expect(menuItems(menu).map((item) => item.text)).toEqual(["Storefront", "Backoffice", "All projects"]),
      );
    });
  });
});

describe("the visit history", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-21T10:00:00Z"));
    vi.stubGlobal("scrollTo", () => {});
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
    localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    localStorage.clear();
  });

  test("a scope page adds its scope, and a doc or thread page its doc or thread with the scope it belongs to, newest first", async () => {
    fakeApi();
    const router = renderAt(`${PROJECT_PATH}/inbox`);
    await screen.findByRole("heading", { level: 1, name: "Inbox" });
    await visit(router, `${PROJECT_PATH}/app`, "2026-10-21T10:01:00Z", shows("Returns PRD"));
    await visit(router, `${PROJECT_PATH}/features/${CHECKOUT}`, "2026-10-21T10:02:00Z", shows("Pay in one step"));
    await visit(router, `${PROJECT_PATH}/docs/${APP_DOC}`, "2026-10-21T10:03:00Z", docShown);
    await visit(router, `${PROJECT_PATH}/docs/${CHECKOUT_DOC}`, "2026-10-21T10:04:00Z", docShown);
    await visit(router, `${PROJECT_PATH}/threads/${CHECKOUT_THREAD}`, "2026-10-21T10:05:00Z", threadShown("Which wallets?"));

    await waitFor(() => expect(storedVisits()).toHaveLength(5));
    expect(storedVisits()).toEqual([
      { scope: CHECKOUT, thread: CHECKOUT_THREAD, at: Date.parse("2026-10-21T10:05:00Z") },
      { scope: CHECKOUT, doc: CHECKOUT_DOC, at: Date.parse("2026-10-21T10:04:00Z") },
      { scope: "app", doc: APP_DOC, at: Date.parse("2026-10-21T10:03:00Z") },
      { scope: CHECKOUT, at: Date.parse("2026-10-21T10:02:00Z") },
      { scope: "app", at: Date.parse("2026-10-21T10:01:00Z") },
    ]);
  });

  test("a place visited again moves to the front with the new time, whatever its query or activity seq", async () => {
    fakeApi();
    const router = renderAt(`${PROJECT_PATH}/inbox`);
    await screen.findByRole("heading", { level: 1, name: "Inbox" });
    await visit(router, `${PROJECT_PATH}/docs/${APP_DOC}?node=p0000001`, "2026-10-21T10:01:00Z", docShown);
    await visit(router, `${PROJECT_PATH}/app`, "2026-10-21T10:02:00Z", shows("Returns PRD"));
    await visit(router, `${PROJECT_PATH}/docs/${APP_DOC}/activity/3`, "2026-10-21T10:03:00Z", shows("The change could not be loaded. It may not exist."));

    await waitFor(() => expect(storedVisits()[0]).toEqual({ scope: "app", doc: APP_DOC, at: Date.parse("2026-10-21T10:03:00Z") }));
    expect(storedVisits()).toEqual([
      { scope: "app", doc: APP_DOC, at: Date.parse("2026-10-21T10:03:00Z") },
      { scope: "app", at: Date.parse("2026-10-21T10:02:00Z") },
    ]);
  });

  test("the Inbox, the Features page and the Threads page add no place", async () => {
    fakeApi();
    const router = renderAt(`${PROJECT_PATH}/inbox`);
    await screen.findByRole("heading", { level: 1, name: "Inbox" });
    await visit(router, `${PROJECT_PATH}/features`, "2026-10-21T10:01:00Z", shows("Features"));
    await visit(router, `${PROJECT_PATH}/threads?scope=${CHECKOUT}`, "2026-10-21T10:02:00Z", shows("Which wallets?"));
    await visit(router, `${PROJECT_PATH}/threads?scope=app`, "2026-10-21T10:03:00Z", shows("No threads here yet."));
    await visit(router, `${PROJECT_PATH}/threads`, "2026-10-21T10:04:00Z", shows("Which wallets?"));

    await settled();
    expect(localStorage.getItem(VISITS_KEY)).toBeNull();
  });

  test("a doc, thread or feature that cannot be loaded adds no place", async () => {
    fakeApi();
    const router = renderAt(`${PROJECT_PATH}/docs/${MISSING}`);
    await screen.findByText("The doc could not be loaded. Try again in a moment.");
    await visit(router, `${PROJECT_PATH}/threads/${MISSING}`, "2026-10-21T10:01:00Z", shows("The thread could not be loaded. Try again in a moment."));
    await visit(router, `${PROJECT_PATH}/features/${MISSING}`, "2026-10-21T10:02:00Z", shows("Feature not found"));

    await settled();
    expect(localStorage.getItem(VISITS_KEY)).toBeNull();
  });

  test("only the newest 50 places are kept", async () => {
    const earlier = Array.from({ length: 50 }, (_, index) => ({
      scope: "app",
      doc: `0192d7a0-5555-7000-8000-1${index.toString(16).padStart(11, "0")}`,
      at: Date.parse("2026-10-20T10:00:00Z") - index,
    }));
    localStorage.setItem(VISITS_KEY, JSON.stringify(earlier));
    fakeApi();
    renderAt(`${PROJECT_PATH}/features/${CHECKOUT}`);
    await waitFor(() => expect(storedVisits()[0]).toEqual({ scope: CHECKOUT, at: Date.parse("2026-10-21T10:00:00Z") }));
    expect(storedVisits()).toEqual([{ scope: CHECKOUT, at: Date.parse("2026-10-21T10:00:00Z") }, ...earlier.slice(0, 49)]);
  });
});

/** Stores `places` as the project's visit history, newest first, a minute apart. */
function seedVisits(...places: { scope: string; doc?: string; thread?: string }[]) {
  const visits = places.map((place, index) => ({ ...place, at: Date.parse("2026-10-21T09:00:00Z") - index * 60_000 }));
  localStorage.setItem(VISITS_KEY, JSON.stringify(visits));
}

/** Opens `path` afresh, as a reload of the page does. */
function reload(path: string) {
  cleanup();
  return renderAt(path);
}

/** The Recent label, a button. */
async function recentLabel() {
  return within(await projectNav()).findByRole("button", { name: "Recent" });
}

/** Recent's label with what it lists. */
async function recent() {
  return (await recentLabel()).parentElement as HTMLElement;
}

/**
 * The rows of the scope tree, from its name down to its Threads row: a link,
 * a pillar's fold button, or an empty pillar's row, each as its shown text and
 * its href when it has one.
 */
async function treeRows() {
  const nav = await projectNav();
  const tree = (await within(nav).findByRole("link", { name: /^Threads \(\d+\)$/ })).parentElement as HTMLElement;
  return Array.from(tree.querySelectorAll<HTMLElement>("a, button, div"))
    .filter((row) => row.matches("a, button") || (row.childElementCount > 0 && !row.querySelector("a, button, div")))
    .map((row) => ({ text: shownText(row), href: row.getAttribute("href") }));
}

/** The tree's first row: the scope's name. */
async function treeHead() {
  return (await treeRows())[0];
}

describe("the sidebar's Recent", () => {
  const sixFeatures = [
    feature(CHECKOUT, "Checkout"),
    feature(WALLETS, "Wallets"),
    feature(SEARCH, "Search"),
    feature(RETURNS, "Returns"),
    feature(GIFT_CARDS, "Gift cards"),
    feature(LOYALTY, "Loyalty"),
  ];

  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
    localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    localStorage.clear();
  });

  test("lists the last 5 features visited, newest first and each once, by name, each opening its page", async () => {
    fakeApi({ home: home({ features: sixFeatures }) });
    seedVisits(
      { scope: CHECKOUT, thread: CHECKOUT_THREAD },
      { scope: "app", doc: APP_DOC },
      { scope: WALLETS },
      { scope: CHECKOUT, doc: CHECKOUT_DOC },
      { scope: SEARCH, doc: "0192d7a0-5555-7000-8000-000000000003" },
      { scope: "app" },
      { scope: RETURNS },
      { scope: GIFT_CARDS, thread: "0192d7a0-6666-7000-8000-000000000002" },
      { scope: LOYALTY },
    );
    renderAt(`${PROJECT_PATH}/inbox`);
    const list = await recent();
    await waitFor(() => expect(links(list)).toHaveLength(5));
    expect(links(list)).toEqual([
      { text: "Checkout", href: `${PROJECT_PATH}/features/${CHECKOUT}` },
      { text: "Wallets", href: `${PROJECT_PATH}/features/${WALLETS}` },
      { text: "Search", href: `${PROJECT_PATH}/features/${SEARCH}` },
      { text: "Returns", href: `${PROJECT_PATH}/features/${RETURNS}` },
      { text: "Gift cards", href: `${PROJECT_PATH}/features/${GIFT_CARDS}` },
    ]);
  });

  test("a visited feature that is gone is skipped, and an older one takes its place", async () => {
    fakeApi({ home: home({ features: sixFeatures }) });
    seedVisits({ scope: GONE }, { scope: CHECKOUT }, { scope: WALLETS }, { scope: SEARCH }, { scope: RETURNS }, { scope: GIFT_CARDS });
    renderAt(`${PROJECT_PATH}/inbox`);
    const list = await recent();
    await waitFor(() => expect(links(list)).toHaveLength(5));
    expect(links(list).map((link) => link.text)).toEqual(["Checkout", "Wallets", "Search", "Returns", "Gift cards"]);
  });

  test("with no feature visited it says No recent features, until one is visited", async () => {
    fakeApi();
    seedVisits({ scope: "app", doc: APP_DOC });
    const router = renderAt(`${PROJECT_PATH}/app`);
    const list = await recent();
    await within(list).findByText("No recent features");
    expect(links(list)).toEqual([]);

    await router.navigate({ href: `${PROJECT_PATH}/features/${CHECKOUT}` });
    await waitFor(async () =>
      expect(links(await recent())).toEqual([{ text: "Checkout", href: `${PROJECT_PATH}/features/${CHECKOUT}` }]),
    );
  });

  test("its label folds and unfolds the list, unfolded at first, and the fold survives a reload", async () => {
    fakeApi();
    seedVisits({ scope: CHECKOUT });
    renderAt(`${PROJECT_PATH}/inbox`);
    await within(await recent()).findByRole("link", { name: "Checkout" });
    expect((await recentLabel()).getAttribute("aria-expanded")).toBe("true");

    fireEvent.click(await recentLabel());
    await waitFor(async () => expect(links(await recent())).toEqual([]));
    reload(`${PROJECT_PATH}/inbox`);
    await waitFor(async () => expect((await recentLabel()).getAttribute("aria-expanded")).toBe("false"));
    expect(links(await recent())).toEqual([]);

    fireEvent.click(await recentLabel());
    await within(await recent()).findByRole("link", { name: "Checkout" });
    reload(`${PROJECT_PATH}/inbox`);
    await within(await recent()).findByRole("link", { name: "Checkout" });
  });
});

describe("the sidebar's scope tree", () => {
  const PLAN = "0192d7a0-5555-7000-8000-000000000011";
  const NOTES = "0192d7a0-5555-7000-8000-000000000012";
  const CART_SCREENS = "0192d7a0-5555-7000-8000-000000000013";
  const TOKENS = "0192d7a0-5555-7000-8000-000000000014";
  const LOAD_TEST = "0192d7a0-5555-7000-8000-000000000015";
  const appTree = home({
    docs: [
      doc(APP_DOC, null, "Returns PRD", { waiting: 2 }),
      doc(NOTES, null, "beta notes"),
      doc(CART_SCREENS, null, "Cart screens", { pillar: "ui", kind: "design" }),
      doc(PLAN, null, "Alpha plan"),
      doc(LOAD_TEST, null, "Load test", { pillar: "research" }),
      doc(TOKENS, null, "Tokens", { pillar: "ui", kind: "design-system" }),
      doc(CHECKOUT_DOC, CHECKOUT, "Pay in one step"),
    ],
    threads: [
      thread("0192d7a0-6666-7000-8000-000000000011", null, "Which carriers?"),
      thread(CHECKOUT_THREAD, CHECKOUT, "Which wallets?"),
      thread("0192d7a0-6666-7000-8000-000000000012", null, "Keep the old URLs?"),
    ],
  });
  const docRow = (id: string, text: string) => ({ text, href: `${PROJECT_PATH}/docs/${id}` });

  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
    localStorage.clear();
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    localStorage.clear();
  });

  test("App's tree: its name, the five pillars with their doc counts and docs by title, the design system first, what waits, and its Threads", async () => {
    fakeApi({ home: appTree });
    renderAt(`${PROJECT_PATH}/app`);
    await waitFor(async () => expect((await treeRows())[1]?.text).toBe("Spec3"));
    expect(await treeRows()).toEqual([
      { text: "App", href: `${PROJECT_PATH}/app` },
      { text: "Spec3", href: null },
      docRow(PLAN, "Alpha plan"),
      docRow(NOTES, "beta notes"),
      docRow(APP_DOC, "Returns PRD2"),
      { text: "UI2", href: null },
      docRow(TOKENS, "Tokens"),
      docRow(CART_SCREENS, "Cart screens"),
      { text: "Architectureempty", href: null },
      { text: "DBempty", href: null },
      { text: "Research1", href: null },
      docRow(LOAD_TEST, "Load test"),
      { text: "Threads (2)", href: `${PROJECT_PATH}/threads?scope=app` },
    ]);
  });

  test("on a feature's thread or doc page it shows that feature, its Threads row filtered to it, then its Board row with the open stories", async () => {
    fakeApi({ home: { ...appTree, features: [feature(CHECKOUT, "Checkout", { total: 3, open: 2, done: 1 })] } });
    const router = renderAt(`${PROJECT_PATH}/threads/${CHECKOUT_THREAD}`);
    await waitFor(async () => expect((await treeHead()).href).toBe(`${PROJECT_PATH}/features/${CHECKOUT}`));
    expect((await treeHead()).text).toMatch(/^Checkout/);
    expect((await treeRows()).slice(-2)).toEqual([
      { text: "Threads (1)", href: `${PROJECT_PATH}/threads?scope=${CHECKOUT}` },
      { text: "Board (2)", href: `${PROJECT_PATH}/features/${CHECKOUT}/board` },
    ]);

    await router.navigate({ href: `${PROJECT_PATH}/docs/${CHECKOUT_DOC}` });
    await waitFor(docShown);
    expect((await treeRows()).filter((row) => row.href?.includes("/docs/"))).toEqual([docRow(CHECKOUT_DOC, "Pay in one step")]);
    expect((await treeHead()).href).toBe(`${PROJECT_PATH}/features/${CHECKOUT}`);
  });

  test("on the Features page it shows the scope visited last", async () => {
    fakeApi({ home: appTree });
    seedVisits({ scope: CHECKOUT, doc: CHECKOUT_DOC }, { scope: "app" });
    renderAt(`${PROJECT_PATH}/features`);
    await waitFor(async () => expect((await treeHead()).href).toBe(`${PROJECT_PATH}/features/${CHECKOUT}`));
  });

  test("with no visit, or with a last visited feature that is gone, it shows App", async () => {
    fakeApi({ home: appTree });
    renderAt(`${PROJECT_PATH}/inbox`);
    expect(await treeHead()).toEqual({ text: "App", href: `${PROJECT_PATH}/app` });

    seedVisits({ scope: GONE }, { scope: CHECKOUT });
    reload(`${PROJECT_PATH}/threads`);
    expect(await treeHead()).toEqual({ text: "App", href: `${PROJECT_PATH}/app` });
  });

  test("a pillar folds on its own, unfolded at first, and its fold survives a reload", async () => {
    fakeApi({ home: appTree });
    renderAt(`${PROJECT_PATH}/app`);
    const nav = await projectNav();
    await within(nav).findByRole("link", { name: "Alpha plan" });
    expect(within(nav).getByRole("button", { name: /^Spec/ }).getAttribute("aria-expanded")).toBe("true");

    fireEvent.click(within(nav).getByRole("button", { name: /^Spec/ }));
    await waitFor(() => expect(within(nav).queryByRole("link", { name: "Alpha plan" })).toBeNull());
    expect(within(nav).getByRole("link", { name: "Tokens" }), "UI stays unfolded").toBeTruthy();

    const router = reload(`${PROJECT_PATH}/app`);
    const reloaded = await projectNav();
    await within(reloaded).findByRole("link", { name: "Tokens" });
    expect(within(reloaded).queryByRole("link", { name: "Alpha plan" }), "Spec is still folded").toBeNull();
    expect(within(reloaded).getByRole("button", { name: /^Spec/ }).getAttribute("aria-expanded")).toBe("false");

    await router.navigate({ href: `${PROJECT_PATH}/features/${CHECKOUT}` });
    await within(await projectNav()).findByRole("link", { name: "Pay in one step" });
  });
});
