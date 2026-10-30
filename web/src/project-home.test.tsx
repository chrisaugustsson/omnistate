// @vitest-environment jsdom
// US-112: the scope pages. A project opens at its App page,
// /w/<workspace id>/projects/<project id>/app, and each feature has its page at
// .../features/<feature id>; both share one layout. The app is rendered with
// its own router on a memory history, the browser's fetch is replaced by a fake
// of the API, and the browser's WebSocket by a fake socket that the test drives.
//
// The pages load GET .../projects/<project>/home (crates/omni-api/tests/
// project_home.rs): its `app.last_activity`, its `features` with each one's
// `last_activity`, its `docs` with each one's `updated_at`, `last_writer_role`
// and `waiting`, and its `threads` with each one's `post_count` and
// `last_post_at`. A section is named by its heading, so it has the role region.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const FEATURE_ID = "0192d7a0-4444-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const APP_PATH = `${PROJECT_PATH}/app`;
const FEATURE_PATH = `${PROJECT_PATH}/features/${FEATURE_ID}`;
const AT = "2026-10-21T10:00:00.000000Z";

let counter = 0;
/** A UUIDv7-shaped ID that no other fixture has. */
function newId() {
  counter += 1;
  return `0192d7a0-3333-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

function doc(pillar: string, kind: string, title: string, fields: { feature?: string; role?: string; waiting?: number } = {}) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: fields.feature ?? null,
    kind,
    pillar,
    title,
    created_at: AT,
    drafts: [],
    claims: [],
    updated_at: AT,
    last_writer_role: fields.role ?? "writer",
    waiting: fields.waiting ?? 0,
  };
}

function thread(
  question: string,
  fields: { feature?: string; createdAt?: string; posts?: number; lastPostAt?: string | null; sealed?: boolean; conclusion?: string | null },
) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: fields.feature ?? null,
    question,
    sealed: fields.sealed ?? false,
    conclusion: fields.conclusion ?? null,
    run_tag: null,
    created_at: fields.createdAt ?? AT,
    post_count: fields.posts ?? 0,
    last_post_at: fields.lastPostAt ?? null,
  };
}

/**
 * A project whose app level has docs in four pillars (none in Architecture)
 * and four threads, and one feature, Checkout, with a doc and a thread of its
 * own. Nothing at the app level has been written yet that counts as activity.
 */
function seededHome() {
  const docs = {
    returns: doc("spec", "html", "Returns PRD", { role: "author", waiting: 2 }),
    checkout: doc("spec", "html", "Checkout PRD", { role: "author" }),
    cart: doc("ui", "design", "Cart screens", { role: "designer" }),
    account: doc("ui", "design", "account screens", { role: "designer" }),
    system: doc("ui", "design-system", "Tokens", { role: "designer" }),
    schema: doc("db", "html", "Orders schema"),
    interviews: doc("research", "html", "Shopper interviews"),
    pay: doc("spec", "html", "Pay in one step", { feature: FEATURE_ID }),
  };
  const threads = {
    // Concluded wins over sealed; its newest post is the newest of all.
    hero: thread("Which hero layout?", {
      createdAt: "2026-10-21T09:00:00.000000Z",
      posts: 3,
      lastPostAt: "2026-10-21T12:00:00.000000Z",
      sealed: true,
      conclusion: "The split hero.",
    }),
    footer: thread("What goes in the footer?", {
      createdAt: "2026-10-21T08:00:00.000000Z",
      posts: 1,
      lastPostAt: "2026-10-21T11:00:00.000000Z",
      sealed: true,
    }),
    // Without posts, the thread counts from its start.
    search: thread("Do we need search?", { createdAt: "2026-10-21T11:30:00.000000Z" }),
    fonts: thread("Which fonts?", {
      createdAt: "2026-10-21T07:00:00.000000Z",
      posts: 2,
      lastPostAt: "2026-10-21T10:00:00.000000Z",
    }),
    wallets: thread("Which wallets?", { feature: FEATURE_ID, posts: 1, lastPostAt: "2026-10-21T13:00:00.000000Z" }),
  };
  const home = {
    project_id: PROJECT_ID,
    app: { last_activity: null as string | null },
    features: [
      {
        feature: {
          id: FEATURE_ID,
          project_id: PROJECT_ID,
          name: "Checkout",
          description: "Pay in one step.\nNo account needed.",
          created_at: AT,
        },
        docs: 1,
        waiting: 0,
        last_activity: "2026-10-21T13:00:00.000000Z" as string | null,
      },
    ],
    docs: Object.values(docs),
    threads: Object.values(threads),
    waiting: { comments: [] },
  };
  return { home, docs, threads };
}

type Home = ReturnType<typeof seededHome>["home"];

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

/** The section named `name` in `container`, once it is there. */
async function section(container: HTMLElement, name: RegExp) {
  return within(container).findByRole("region", { name });
}

/** The names of the sections of `container`, in order. */
function sectionNames(container: HTMLElement) {
  return within(container)
    .getAllByRole("region")
    .map((region) => (region.querySelector("h2")?.textContent ?? "").trim());
}

/** The link of the row that shows `title`. */
function rowLink(container: HTMLElement, title: string) {
  const link = within(container).getByText(title).closest("a");
  expect(link, `the row of ${title} is a link`).toBeTruthy();
  return link as HTMLAnchorElement;
}

/** The titles of the rows of `container`, in order. */
function rowTitles(container: HTMLElement) {
  return within(container)
    .getAllByRole("listitem")
    .map((item) => (item.querySelector("a span")?.textContent ?? "").trim());
}

describe("the scope pages", () => {
  beforeEach(() => {
    // Three hours after the hero thread's newest post, five after the docs were written.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-21T15:00:00.000Z"));
    vi.stubGlobal("scrollTo", () => {});
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  test("a project opens at its App page", async () => {
    fakeApi(seededHome().home);
    const router = renderAt(`/w/${WORKSPACE_ID}`);
    const project = await screen.findByRole("link", { name: "Storefront" });
    expect(project.getAttribute("href")).toBe(PROJECT_PATH);
    fireEvent.click(project);
    await waitFor(() => expect(router.state.location.pathname).toBe(APP_PATH));
    expect(await within(await page()).findByRole("heading", { level: 1, name: "Storefront" })).toBeTruthy();
  });

  test("the App page's header has the project's name and no description, and says when nothing happened yet", async () => {
    fakeApi(seededHome().home);
    renderAt(APP_PATH);
    const main = await page();
    const heading = await within(main).findByRole("heading", { level: 1, name: "Storefront" });
    const header = heading.closest("header") ?? heading.parentElement!;
    expect(header.textContent).toContain("No activity yet");
    expect(main.textContent).not.toContain("The app as intended now.");
  });

  test("the five pillars come in order, an empty one saying No docs yet", async () => {
    fakeApi(seededHome().home);
    renderAt(APP_PATH);
    const main = await page();
    await section(main, /^spec\b/i);
    expect(sectionNames(main)).toEqual(["Spec", "UI", "Architecture", "DB", "Research", "Threads"]);
    const architecture = await section(main, /^architecture\b/i);
    expect(architecture.textContent).toContain("No docs yet");
    expect(within(architecture).queryAllByRole("listitem")).toEqual([]);
  });

  test("the chips count each pillar's docs and jump to its section", async () => {
    fakeApi(seededHome().home);
    renderAt(APP_PATH);
    const chips = await screen.findByRole("navigation", { name: /^pillars$/i });
    const links = within(chips).getAllByRole("link");
    expect(links.map((link) => [link.getAttribute("href"), (link.textContent ?? "").replace(/\s+/g, " ").trim()])).toEqual([
      ["#spec", "Spec2"],
      ["#ui", "UI3"],
      ["#architecture", "Architecture0"],
      ["#db", "DB1"],
      ["#research", "Research1"],
    ]);
  });

  test("a pillar lists its docs by title, the design system first", async () => {
    fakeApi(seededHome().home);
    renderAt(APP_PATH);
    const main = await page();
    const spec = await section(main, /^spec\b/i);
    await within(spec).findByText("Returns PRD");
    expect(rowTitles(spec), "the feature's doc is not the app's").toEqual(["Checkout PRD", "Returns PRD"]);
    expect(rowTitles(await section(main, /^ui\b/i))).toEqual(["Tokens", "account screens", "Cart screens"]);
  });

  test("a doc's row shows its kind, the role that last wrote it and how many comments wait, and opens the doc", async () => {
    const { home, docs } = seededHome();
    fakeApi(home);
    const router = renderAt(APP_PATH);
    const main = await page();
    const spec = await section(main, /^spec\b/i);
    const returns = await waitFor(() => rowLink(spec, "Returns PRD"));
    expect(returns.textContent).toMatch(/html/i);
    expect(returns.textContent).toContain("author");
    expect(within(returns).getByText("2", { exact: true }), "two comments wait").toBeTruthy();
    expect(within(rowLink(spec, "Checkout PRD")).queryByText("0", { exact: true }), "no badge for none").toBeNull();
    const ui = await section(main, /^ui\b/i);
    expect(rowLink(ui, "Tokens").textContent).toMatch(/design system/i);
    expect(rowLink(ui, "Cart screens").textContent).toContain("designer");
    expect(within(returns).getByText(/^5 h ago · author$/).getAttribute("title"), "the absolute time in a tooltip").toContain("2026");

    expect(returns.getAttribute("href")).toBe(`${PROJECT_PATH}/docs/${docs.returns.id}`);
    fireEvent.click(returns);
    await waitFor(() => expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/docs/${docs.returns.id}`));
  });

  test("the threads come newest post first, a thread without posts at its start, each with its count and one flag", async () => {
    const { home, threads } = seededHome();
    fakeApi(home);
    renderAt(APP_PATH);
    const main = await page();
    const listed = await section(main, /^threads\b/i);
    await within(listed).findByText("Which hero layout?");
    expect(rowTitles(listed), "the feature's thread is not the app's").toEqual([
      "Which hero layout?",
      "Do we need search?",
      "What goes in the footer?",
      "Which fonts?",
    ]);
    const flags = (title: string) => {
      const text = rowLink(listed, title).textContent ?? "";
      return ["Concluded", "Sealed", "Open"].filter((flag) => text.includes(flag));
    };
    expect(flags("Which hero layout?")).toEqual(["Concluded"]);
    expect(flags("What goes in the footer?")).toEqual(["Sealed"]);
    expect(flags("Do we need search?")).toEqual(["Open"]);
    expect(rowLink(listed, "Which hero layout?").textContent).toMatch(/\b3 posts/);
    expect(within(rowLink(listed, "Which hero layout?")).getByText(/^3 h ago$/).getAttribute("title"), "the absolute time in a tooltip").toContain("2026");
    expect(rowLink(listed, "Do we need search?").textContent).toMatch(/\b0 posts/);
    expect(rowLink(listed, "Which hero layout?").getAttribute("href")).toBe(`${PROJECT_PATH}/threads/${threads.hero.id}`);
  });

  test("a feature's page has the feature's name and description, and only its own docs and threads", async () => {
    const { home } = seededHome();
    home.threads = home.threads.filter((listed) => listed.feature_id === null);
    fakeApi(home);
    renderAt(FEATURE_PATH);
    const main = await page();
    const heading = await within(main).findByRole("heading", { level: 1, name: "Checkout" });
    const header = heading.closest("header") ?? heading.parentElement!;
    const description = within(header).getByText(/Pay in one step\./);
    expect(description.textContent).toBe("Pay in one step.\nNo account needed.");
    expect(description.className, "the line break is kept").toMatch(/\bwhitespace-pre-(line|wrap)\b/);
    const activity = within(header).getByText(/^Last change 2 h ago$/);
    expect(activity.getAttribute("title"), "the absolute time in a tooltip").toContain("2026");
    expect(rowTitles(await section(main, /^spec\b/i))).toEqual(["Pay in one step"]);
    expect((await section(main, /^ui\b/i)).textContent).toContain("No docs yet");
    expect((await section(main, /^threads\b/i)).textContent).toContain("No threads yet");
  });

  test("the page updates live as the project's events come in", async () => {
    const api = fakeApi(seededHome().home);
    renderAt(APP_PATH);
    const main = await page();
    const research = await section(main, /^research\b/i);
    await within(research).findByText("Shopper interviews");

    await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
    const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
    socket.open();
    await waitFor(() =>
      expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })),
    );
    socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 10 });

    // An agent starts a doc in Research.
    const started = doc("research", "html", "Competitor notes");
    api.home = { ...api.home, docs: [...api.home.docs, started] };
    socket.receive({
      type: "event",
      seq: 11,
      event: "doc.created",
      project_id: PROJECT_ID,
      at: AT,
      data: { doc_id: started.id },
    });
    await waitFor(() => expect(rowTitles(research)).toEqual(["Competitor notes", "Shopper interviews"]));
  });
});
