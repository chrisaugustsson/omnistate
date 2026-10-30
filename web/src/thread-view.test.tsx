// @vitest-environment jsdom
// US-114: the thread page (/…/threads/<thread>) and the Threads page
// (/…/threads?scope=app|<feature>). The app is rendered with its own router on
// a memory history, fetch is replaced by a fake of the API and WebSocket by a
// fake socket that the test drives.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const CHECKOUT_ID = "0192d7a0-4444-7000-8000-000000000001";
const SEARCH_ID = "0192d7a0-4444-7000-8000-000000000002";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const THREADS_PATH = `${PROJECT_PATH}/threads`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const NOW = "2026-10-21T15:00:00.000Z";
const HUMAN = { kind: "human", user_id: USER.id };

function agent(role: string, model: string) {
  return { kind: "agent", run: "run-1", role, model, harness: "claude-code" };
}

let counter = 0;
/** A UUIDv7-shaped ID that no other fixture has. */
function newId() {
  counter += 1;
  return `0192d7a0-3333-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

function post(threadId: string, author: Record<string, unknown>, summary: string, bodyHtml: string, createdAt: string) {
  return {
    id: newId(),
    thread_id: threadId,
    author,
    summary,
    body: bodyHtml.replace(/<[^>]+>/g, ""),
    body_html: bodyHtml,
    links: [] as Record<string, unknown>[],
    attachments: [] as string[],
    created_at: createdAt,
  };
}

type Post = ReturnType<typeof post>;

function thread(
  question: string,
  fields: {
    feature?: string;
    createdAt?: string;
    posts?: number;
    lastPostAt?: string | null;
    sealed?: boolean;
    conclusion?: string | null;
  } = {},
) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: fields.feature ?? null,
    question,
    sealed: fields.sealed ?? false,
    conclusion: fields.conclusion ?? null,
    run_tag: null as string | null,
    created_at: fields.createdAt ?? "2026-10-21T08:00:00.000Z",
    post_count: fields.posts ?? 0,
    last_post_at: fields.lastPostAt ?? null,
  };
}

type HomeThread = ReturnType<typeof thread>;
type Opened = HomeThread & { posts: Post[] };

function feature(id: string, name: string) {
  return {
    feature: { id, project_id: PROJECT_ID, name, description: "", created_at: "2026-10-20T08:00:00.000Z" },
    docs: 0,
    waiting: 0,
    last_activity: null,
  };
}

function home(threads: HomeThread[]) {
  return {
    project_id: PROJECT_ID,
    app: { last_activity: null },
    features: [feature(CHECKOUT_ID, "Checkout"), feature(SEARCH_ID, "Search")],
    docs: [],
    threads,
    waiting: { comments: [] },
  };
}

type Call = { method: string; path: string; body: Record<string, unknown> | null };

/**
 * A fake of the API: the auth routes, the project's home and the opened
 * thread, which the test may change. A post is stored and answered 201 unless
 * `failPosts` is set, then 500.
 */
function fakeApi(threads: HomeThread[], opened: Opened | null = null) {
  const api = { home: home(threads), opened, failPosts: false, calls: [] as Call[] };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url, "http://localhost:3000").pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    const text = typeof init?.body === "string" ? init.body : input instanceof Request ? await input.clone().text() : "";
    const body = text ? (JSON.parse(text) as Record<string, unknown>) : null;
    api.calls.push({ method, path, body });
    switch (`${method} ${path}`) {
      case "GET /api/v1/auth/me":
        return json(200, { user: USER });
      case "GET /api/v1/auth/workspaces":
        return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
      case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
        return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: "2026-10-20T08:00:00.000Z" }] });
      case `GET ${API}/home`:
        return json(200, api.home);
      default:
        break;
    }
    const current = api.opened;
    if (current && method === "GET" && path === `${API}/threads/${current.id}`) {
      return json(200, { thread: current });
    }
    if (current && method === "POST" && path === `${API}/threads/${current.id}/posts`) {
      if (api.failPosts) return json(500, { error: "internal" });
      const created = post(current.id, HUMAN, String(body?.summary), `<p>${String(body?.body)}</p>`, NOW);
      api.opened = { ...current, posts: [...current.posts, created] };
      return json(201, { post: created });
    }
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

/** Opens the live socket and confirms the subscription to the project. */
async function openSocket() {
  await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
  const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
  socket.open();
  await waitFor(() =>
    expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })),
  );
  socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 20 });
  return socket;
}

function postCreated(seq: number, threadId: string, postId: string) {
  return {
    type: "event",
    seq,
    event: "post.created",
    project_id: PROJECT_ID,
    at: NOW,
    data: { post_id: postId, thread_id: threadId },
  };
}

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

/** The posts of the thread page, in order. */
function postItems(main: HTMLElement) {
  return Array.from(main.querySelectorAll<HTMLElement>('li[id^="post-"]'));
}

/** The summaries of the thread page's posts, in order. */
function postSummaries(main: HTMLElement) {
  return postItems(main).map((item) => within(item).getByRole("heading", { level: 3 }).textContent);
}

function summaryField() {
  return screen.getByRole("textbox", { name: "Summary" });
}

function bodyField() {
  return screen.getByRole("textbox", { name: "Body" });
}

function postButton() {
  return screen.getByRole("button", { name: /^post\b/i }) as HTMLButtonElement;
}

/** A Threads page row's link, by the question it shows. */
function threadRow(main: HTMLElement, question: string) {
  const link = within(main).getByText(question).closest("a");
  expect(link, `the row of ${question} is a link`).toBeTruthy();
  return link as HTMLAnchorElement;
}

/** The questions of the Threads page's rows, in order. */
function rowQuestions(main: HTMLElement) {
  return within(main)
    .getAllByRole("listitem")
    .map((item) => (item.querySelector("a span")?.textContent ?? "").trim());
}

describe("threads in the web app", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(NOW));
    vi.stubGlobal("scrollTo", () => {});
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  describe("the thread page", () => {
    /** A thread of Checkout with an agent's post and then one of yours. */
    function openedThread(fields: Partial<Opened> = {}): Opened {
      const listed = thread("Which wallets do we take?", { feature: CHECKOUT_ID });
      const posts = [
        post(listed.id, agent("critic", "claude-opus-4"), "Apple Pay covers most phones", "<p>Seven of ten visits are <strong>iPhones</strong>.</p>", "2026-10-21T12:00:00.000Z"),
        post(listed.id, HUMAN, "Add Swish too", "<p>Most Swedish shoppers use it.</p>", "2026-10-21T14:00:00.000Z"),
      ];
      return { ...listed, posts, ...fields };
    }

    test("the posts come oldest first, each with its summary as a heading, its Markdown body, who wrote it and when", async () => {
      const opened = openedThread();
      fakeApi([opened], opened);
      renderAt(`${THREADS_PATH}/${opened.id}`);
      const main = await screen.findByRole("main");
      await within(main).findByRole("heading", { level: 1, name: "Which wallets do we take?" });

      await waitFor(() => expect(postSummaries(main)).toEqual(["Apple Pay covers most phones", "Add Swish too"]));
      const [first, second] = postItems(main);
      expect(within(first).getByText("iPhones").tagName).toBe("STRONG");
      expect(first.textContent).toContain("critic · claude-opus-4");
      expect(within(first).getByText("3 h ago").getAttribute("title")).toContain("2026");
      expect(second.textContent).toContain("You");
      expect(second.textContent).toContain("Most Swedish shoppers use it.");
      expect(second.textContent).toContain("1 h ago");
    });

    test("a post that arrives is added at the bottom", async () => {
      const opened = openedThread();
      const api = fakeApi([opened], opened);
      renderAt(`${THREADS_PATH}/${opened.id}`);
      const main = await screen.findByRole("main");
      await waitFor(() => expect(postItems(main)).toHaveLength(2));
      const socket = await openSocket();

      const late = post(opened.id, agent("designer", "gpt-5"), "Klarna is wanted in Germany", "<p>Half of the orders there.</p>", "2026-10-21T14:59:00.000Z");
      api.opened = { ...opened, posts: [...opened.posts, late] };
      socket.receive(postCreated(21, opened.id, late.id));

      await waitFor(() =>
        expect(postSummaries(main)).toEqual(["Apple Pay covers most phones", "Add Swish too", "Klarna is wanted in Germany"]),
      );
    });

    test("the conclusion shows once written, and nothing of criteria, phases, directives or decisions", async () => {
      // An old API body still carrying the fields that v2 dropped.
      const opened = {
        ...openedThread({ conclusion: "We take Apple Pay and Swish." }),
        criteria: ["Readable at 320 px"],
        phases: [{ number: 2, sealed: true, started_at: "2026-10-21T09:00:00.000Z", posted_by: [] }],
        decision: { id: newId(), status: "proposed", direction: "Take every wallet", options: [] },
      };
      fakeApi([opened], opened);
      renderAt(`${THREADS_PATH}/${opened.id}`);
      const main = await screen.findByRole("main");

      const conclusion = await within(main).findByRole("region", { name: "Conclusion" });
      expect(conclusion.textContent).toContain("We take Apple Pay and Swish.");
      expect(main.textContent).not.toContain("Readable at 320 px");
      expect(main.textContent).not.toMatch(/phase/i);
      expect(main.textContent).not.toMatch(/directive/i);
      expect(main.textContent).not.toContain("Take every wallet");
      expect(within(main).queryByRole("button", { name: /approve|reject/i })).toBeNull();
    });

    test("a thread without a conclusion shows none", async () => {
      const opened = openedThread();
      fakeApi([opened], opened);
      renderAt(`${THREADS_PATH}/${opened.id}`);
      const main = await screen.findByRole("main");
      await waitFor(() => expect(postItems(main)).toHaveLength(2));
      expect(within(main).queryByRole("region", { name: "Conclusion" })).toBeNull();
    });

    test("Post stays off until both the summary and the body have text", async () => {
      const opened = openedThread();
      fakeApi([opened], opened);
      renderAt(`${THREADS_PATH}/${opened.id}`);
      await screen.findByRole("textbox", { name: "Summary" });

      expect(postButton().disabled).toBe(true);
      fireEvent.change(summaryField(), { target: { value: "Add Klarna" } });
      expect(postButton().disabled, "the body is empty").toBe(true);
      fireEvent.change(bodyField(), { target: { value: "  \n " } });
      expect(postButton().disabled, "the body is only whitespace").toBe(true);
      fireEvent.change(bodyField(), { target: { value: "For Germany." } });
      fireEvent.change(summaryField(), { target: { value: "   " } });
      expect(postButton().disabled, "the summary is only whitespace").toBe(true);
      fireEvent.change(summaryField(), { target: { value: "Add Klarna" } });
      expect(postButton().disabled).toBe(false);
    });

    test("your post is sent as written, the fields clear and it shows once at the bottom", async () => {
      const opened = openedThread();
      const api = fakeApi([opened], opened);
      renderAt(`${THREADS_PATH}/${opened.id}`);
      const main = await screen.findByRole("main");
      await waitFor(() => expect(postItems(main)).toHaveLength(2));
      const socket = await openSocket();

      fireEvent.change(summaryField(), { target: { value: " Add Klarna " } });
      fireEvent.change(bodyField(), { target: { value: "- Germany\n- Austria\n" } });
      fireEvent.click(postButton());

      await waitFor(() => expect((summaryField() as HTMLInputElement).value).toBe(""));
      expect((bodyField() as HTMLTextAreaElement).value).toBe("");
      const sent = api.calls.filter((call) => call.method === "POST" && call.path === `${API}/threads/${opened.id}/posts`);
      expect(sent.map((call) => call.body)).toEqual([{ summary: " Add Klarna ", body: "- Germany\n- Austria\n" }]);

      await waitFor(() => expect(postSummaries(main)).toEqual(["Apple Pay covers most phones", "Add Swish too", " Add Klarna "]));

      // The post's own live event comes in too, and the thread is read again.
      const reads = () => api.calls.filter((call) => call.method === "GET" && call.path === `${API}/threads/${opened.id}`).length;
      const readsBefore = reads();
      const mine = api.opened!.posts[2];
      socket.receive(postCreated(21, opened.id, mine.id));
      await waitFor(() => expect(reads()).toBe(readsBefore + 1));
      expect(postSummaries(main)).toEqual(["Apple Pay covers most phones", "Add Swish too", " Add Klarna "]);
    });

    test("a post that cannot be sent keeps your text and says so", async () => {
      const opened = openedThread();
      const api = fakeApi([opened], opened);
      api.failPosts = true;
      renderAt(`${THREADS_PATH}/${opened.id}`);
      const main = await screen.findByRole("main");
      await waitFor(() => expect(postItems(main)).toHaveLength(2));

      fireEvent.change(summaryField(), { target: { value: "Add Klarna" } });
      fireEvent.change(bodyField(), { target: { value: "For Germany." } });
      fireEvent.click(postButton());

      const alert = await within(main).findByRole("alert");
      expect(alert.textContent).toContain("could not be sent");
      expect((summaryField() as HTMLInputElement).value).toBe("Add Klarna");
      expect((bodyField() as HTMLTextAreaElement).value).toBe("For Germany.");
      expect(postItems(main)).toHaveLength(2);
    });

    test("a feature's thread: Checkout › Threads › question, linking the feature's page and its threads", async () => {
      const opened = openedThread();
      fakeApi([opened], opened);
      renderAt(`${THREADS_PATH}/${opened.id}`);
      const trail = await screen.findByRole("navigation", { name: "Breadcrumb" });

      await waitFor(() => expect(within(trail).getByRole("link", { name: "Checkout" })).toBeTruthy());
      expect(Array.from(trail.querySelectorAll("li"), (item) => item.textContent)).toEqual([
        "Checkout",
        "Threads",
        "Which wallets do we take?",
      ]);
      expect(within(trail).getByRole("link", { name: "Checkout" }).getAttribute("href")).toBe(
        `${PROJECT_PATH}/features/${CHECKOUT_ID}`,
      );
      expect(within(trail).getByRole("link", { name: "Threads" }).getAttribute("href")).toBe(
        `${THREADS_PATH}?scope=${CHECKOUT_ID}`,
      );
    });

    test("an app-level thread: App › Threads › question, linking the App page and the app's threads", async () => {
      const opened = { ...thread("Which fonts?"), posts: [] as Post[] };
      fakeApi([opened], opened);
      renderAt(`${THREADS_PATH}/${opened.id}`);
      const trail = await screen.findByRole("navigation", { name: "Breadcrumb" });

      expect(Array.from(trail.querySelectorAll("li"), (item) => item.textContent)).toEqual(["App", "Threads", "Which fonts?"]);
      expect(within(trail).getByRole("link", { name: "App" }).getAttribute("href")).toBe(`${PROJECT_PATH}/app`);
      expect(within(trail).getByRole("link", { name: "Threads" }).getAttribute("href")).toBe(`${THREADS_PATH}?scope=app`);
    });

    test("#post-<id> scrolls to that post and highlights it", async () => {
      const opened = openedThread();
      const scrolled: Element[] = [];
      Element.prototype.scrollIntoView = vi.fn(function (this: Element) {
        scrolled.push(this);
      });
      fakeApi([opened], opened);
      const [first, second] = opened.posts;
      renderAt(`${THREADS_PATH}/${opened.id}#post-${first.id}`);
      const main = await screen.findByRole("main");

      await waitFor(() => expect(scrolled.map((element) => element.id)).toContain(`post-${first.id}`));
      expect(scrolled.map((element) => element.id)).not.toContain(`post-${second.id}`);
      const [shownFirst, shownSecond] = postItems(main);
      expect(shownFirst.querySelector("[data-highlighted]"), "the linked post is highlighted").toBeTruthy();
      expect(shownSecond.querySelector("[data-highlighted]"), "another post is not").toBeNull();
    });
  });

  describe("the Threads page", () => {
    test("a sealed thread with a conclusion shows both Sealed and Concluded", async () => {
      const both = thread("Sealed and concluded?", {
        feature: CHECKOUT_ID,
        posts: 1,
        lastPostAt: "2026-10-21T14:57:00.000Z",
        sealed: true,
        conclusion: "We pick sealed and concluded.",
      });
      fakeApi([both]);
      renderAt(THREADS_PATH);
      const main = await screen.findByRole("main");

      const row = await waitFor(() => threadRow(main, "Sealed and concluded?"));
      expect(within(row).getByText("Sealed")).toBeTruthy();
      expect(within(row).getByText("Concluded")).toBeTruthy();
    });

    test("the threads of every scope come newest post first, a thread without posts at its start, each with its scope and post count", async () => {
      const fonts = thread("Which fonts?", { posts: 3, lastPostAt: "2026-10-21T11:00:00.000Z" });
      const wallets = thread("Which wallets?", { feature: CHECKOUT_ID, posts: 1, lastPostAt: "2026-10-21T13:00:00.000Z" });
      const footer = thread("What goes in the footer?", { createdAt: "2026-10-21T12:00:00.000Z" });
      const ranking = thread("How do we rank results?", {
        feature: SEARCH_ID,
        createdAt: "2026-10-21T07:00:00.000Z",
        posts: 2,
        lastPostAt: "2026-10-21T10:00:00.000Z",
      });
      fakeApi([fonts, wallets, footer, ranking]);
      renderAt(THREADS_PATH);
      const main = await screen.findByRole("main");

      await waitFor(() =>
        expect(rowQuestions(main)).toEqual(["Which wallets?", "What goes in the footer?", "Which fonts?", "How do we rank results?"]),
      );
      expect(threadRow(main, "Which wallets?").textContent).toContain("Checkout");
      expect(threadRow(main, "Which wallets?").textContent).toContain("1 post");
      expect(threadRow(main, "Which fonts?").textContent).toContain("App");
      expect(threadRow(main, "Which fonts?").textContent).toContain("3 posts");
      expect(threadRow(main, "What goes in the footer?").textContent).toContain("0 posts");
      expect(threadRow(main, "How do we rank results?").textContent).toContain("Search");
      expect(threadRow(main, "Which wallets?").getAttribute("href")).toBe(`${THREADS_PATH}/${wallets.id}`);
    });

    test("?scope=app lists the app's threads only", async () => {
      const fonts = thread("Which fonts?", { posts: 1, lastPostAt: "2026-10-21T11:00:00.000Z" });
      const wallets = thread("Which wallets?", { feature: CHECKOUT_ID, posts: 1, lastPostAt: "2026-10-21T13:00:00.000Z" });
      fakeApi([fonts, wallets]);
      renderAt(`${THREADS_PATH}?scope=app`);
      const main = await screen.findByRole("main");

      await waitFor(() => expect(rowQuestions(main)).toEqual(["Which fonts?"]));
      expect(within(main).getByRole("button", { name: "Scope" }).textContent).toBe("App");
    });

    test("choosing a feature in the scope filter narrows the list and keeps it in ?scope=, All scopes takes it out", async () => {
      const fonts = thread("Which fonts?", { posts: 1, lastPostAt: "2026-10-21T11:00:00.000Z" });
      const wallets = thread("Which wallets?", { feature: CHECKOUT_ID, posts: 1, lastPostAt: "2026-10-21T13:00:00.000Z" });
      const ranking = thread("How do we rank results?", { feature: SEARCH_ID, posts: 1, lastPostAt: "2026-10-21T10:00:00.000Z" });
      fakeApi([fonts, wallets, ranking]);
      const router = renderAt(THREADS_PATH);
      const main = await screen.findByRole("main");
      await waitFor(() => expect(rowQuestions(main)).toHaveLength(3));

      fireEvent.click(within(main).getByRole("button", { name: "Scope" }));
      fireEvent.change(within(main).getByRole("combobox", { name: "Search scopes" }), { target: { value: "check" } });
      expect(within(main).getAllByRole("option").map((option) => option.textContent)).toEqual(["Checkout"]);
      fireEvent.click(within(main).getByRole("option", { name: "Checkout" }));

      await waitFor(() => expect(router.state.location.search).toEqual({ scope: CHECKOUT_ID }));
      await waitFor(() => expect(rowQuestions(main)).toEqual(["Which wallets?"]));

      fireEvent.click(within(main).getByRole("button", { name: "Scope" }));
      fireEvent.click(within(main).getByRole("option", { name: "All scopes" }));
      await waitFor(() => expect(router.state.location.search).toEqual({}));
      expect(router.state.location.href).not.toContain("scope=");
      await waitFor(() => expect(rowQuestions(main)).toEqual(["Which wallets?", "Which fonts?", "How do we rank results?"]));
    });

    test("a new post moves its thread to the top and counts it", async () => {
      const fonts = thread("Which fonts?", { posts: 1, lastPostAt: "2026-10-21T11:00:00.000Z" });
      const wallets = thread("Which wallets?", { feature: CHECKOUT_ID, posts: 1, lastPostAt: "2026-10-21T13:00:00.000Z" });
      const api = fakeApi([fonts, wallets]);
      renderAt(THREADS_PATH);
      const main = await screen.findByRole("main");
      await waitFor(() => expect(rowQuestions(main)).toEqual(["Which wallets?", "Which fonts?"]));
      const socket = await openSocket();

      api.home = home([{ ...fonts, post_count: 2, last_post_at: "2026-10-21T14:30:00.000Z" }, wallets]);
      socket.receive(postCreated(21, fonts.id, newId()));

      await waitFor(() => expect(rowQuestions(main)).toEqual(["Which fonts?", "Which wallets?"]));
      expect(threadRow(main, "Which fonts?").textContent).toContain("2 posts");
    });
  });
});
