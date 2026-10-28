// @vitest-environment jsdom
// US-040: the project home, the reviewer's page of one project. It is
// /w/<workspace id>/projects/<project id>, and the workspace page links each
// project there. The app is rendered with its own router on a memory history,
// the browser's fetch is replaced by a fake of the API, and the browser's
// WebSocket by a fake socket that the test drives.
//
// The page loads GET /api/v1/w/<workspace>/projects/<project>/home
// (crates/omni-api/tests/project_home.rs), which answers {project_id, docs,
// changesets, threads, waiting: {decisions, changesets, comments,
// questions}}, each item the body that the API's own read of it answers (Doc,
// Changeset, Thread, Decision, Comment, Post). The page shows:
//
// - a section named "Docs" in which each pillar that has docs is a group
//   (a section or group named "Spec", "UI", "Architecture", "DB" or
//   "Research", US-101) listing the titles of that pillar's docs;
// - a section named "Change sets" listing each change set's title with its
//   status, one list item (or table row) per change set;
// - a section named "Threads" listing each thread's question;
// - a section named "Waiting for you" listing what `waiting` holds: each
//   decision's direction, each change set's title, each comment's text and
//   each question's summary, and nothing else.
//
// A section is named by its heading (`<section aria-labelledby>`), so it has
// the role region. The page subscribes to the project's live events through
// the workspace's live client (src/live/context.tsx) and loads the home again
// when an event comes, so items appear and disappear without a reload.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const HOME_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T10:00:00Z";
const AGENT = { kind: "agent", run: "run-1", role: "critic", model: "test-model", harness: "test-harness" };
const HUMAN = { kind: "human", user_id: USER.id };

let counter = 0;
/** A UUIDv7-shaped ID that no other fixture has. */
function newId() {
  counter += 1;
  return `0192d7a0-3333-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

function doc(pillar: string, title: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    kind: pillar === "ui" ? "design" : "html",
    pillar,
    title,
    created_at: AT,
    mainline_version_id: null,
    drafts: [],
    claims: [],
  };
}

function changeset(title: string, status: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    title,
    description: "",
    status,
    run_tag: "run-1",
    created_at: AT,
    history: [],
    approved_versions: [],
    decisions: [],
  };
}

function thread(changesetId: string, question: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    changeset_id: changesetId,
    question,
    criteria: ["Readable at 320 px"],
    run_tag: "run-1",
    created_at: AT,
    phases: [{ number: 1, sealed: false, started_at: AT, posted_by: [] }],
    decision: null,
  };
}

function decision(threadId: string, changesetId: string, direction: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    thread_id: threadId,
    changeset_id: changesetId,
    status: "proposed",
    author: AGENT,
    direction,
    chosen: "Split hero",
    criteria: ["Readable at 320 px"],
    options: [{ name: "Split hero", scores: { "Readable at 320 px": 8 }, lost_because: null }],
    assumptions: [],
    open_questions: [],
    dissent: [],
    posts: [],
    rejection_reason: null,
    created_at: AT,
  };
}

function comment(docId: string, changesetId: string, text: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    doc_id: docId,
    version_id: newId(),
    node: "abcdefgh",
    changeset_id: changesetId,
    author: HUMAN,
    text,
    status: "addressed",
    addressed_in: newId(),
    replies: [],
    node_status: "current",
    compared_to: null,
    created_at: AT,
  };
}

function question(threadId: string, summary: string) {
  return {
    id: newId(),
    thread_id: threadId,
    phase: 1,
    type: "question",
    author: AGENT,
    summary,
    body: "Which breakpoint matters most?",
    body_html: "<p>Which breakpoint matters most?</p>",
    links: [],
    attachments: [],
    confidence: null,
    replaces: null,
    created_at: AT,
  };
}

/** A project with docs of four pillars, change sets of four statuses, two threads and one item of each kind waiting. */
function seededHome() {
  const prdHero = doc("spec", "Hero PRD");
  const prdCheckout = doc("spec", "Checkout PRD");
  const dbSchema = doc("db", "Orders schema");
  const uiDesign = doc("ui", "Storefront design");
  const interviews = doc("research", "Shopper interviews");
  const open = changeset("Draft the footer", "open");
  const inReview = changeset("Rework the hero", "in_review");
  const rejected = changeset("Carousel hero", "rejected");
  const merged = changeset("First storefront", "merged");
  const heroThread = thread(inReview.id, "Which hero layout?");
  const footerThread = thread(open.id, "Should the footer hold the newsletter form?");
  return {
    project_id: PROJECT_ID,
    docs: [prdHero, prdCheckout, dbSchema, uiDesign, interviews],
    changesets: [open, inReview, rejected, merged],
    threads: [heroThread, footerThread],
    waiting: {
      decisions: [decision(heroThread.id, inReview.id, "Use the split hero with the copy on the left")],
      changesets: [inReview],
      comments: [comment(prdHero.id, inReview.id, "The hero copy is shorter now")],
      questions: [question(footerThread.id, "Does the newsletter form need a consent box?")],
    },
  };
}

type Home = ReturnType<typeof seededHome>;

/** A fake of the API: the auth routes, the workspace's one project and its home, which the test may change. */
function fakeApi(home: Home) {
  const api = { home };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
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

/** The page's section (or group) named `name`, inside `container` when given. */
async function section(name: RegExp, container?: HTMLElement) {
  const scope = container ? within(container) : screen;
  return waitFor(() => {
    const found = scope.queryByRole("region", { name }) ?? scope.queryByRole("group", { name });
    expect(found, `a section named ${name}`).toBeTruthy();
    return found as HTMLElement;
  });
}

const DOCS = /^docs\b/i;
const WAITING = /^waiting for you\b/i;

describe("the project home", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  test("the workspace page links each project to its home", async () => {
    fakeApi(seededHome());
    const router = renderAt(`/w/${WORKSPACE_ID}`);
    const link = await screen.findByRole("link", { name: /storefront/i });
    expect(link.getAttribute("href")).toBe(HOME_PATH);
    fireEvent.click(link);
    await waitFor(() => expect(router.state.location.pathname).toBe(HOME_PATH));
    await section(DOCS);
  });

  test("the docs are listed grouped by pillar", async () => {
    fakeApi(seededHome());
    renderAt(HOME_PATH);
    const docs = await section(DOCS);

    const spec = await section(/^spec\b/i, docs);
    expect(within(spec).getByText("Hero PRD")).toBeTruthy();
    expect(within(spec).getByText("Checkout PRD")).toBeTruthy();
    expect(within(spec).queryByText("Orders schema")).toBeNull();
    expect(within(spec).queryByText("Storefront design")).toBeNull();
    expect(within(spec).queryByText("Shopper interviews")).toBeNull();

    const research = await section(/^research\b/i, docs);
    expect(within(research).getByText("Shopper interviews")).toBeTruthy();
    expect(within(research).queryByText("Hero PRD")).toBeNull();

    const db = await section(/^(db|database)\b/i, docs);
    expect(within(db).getByText("Orders schema")).toBeTruthy();
    expect(within(db).queryByText("Hero PRD")).toBeNull();

    const ui = await section(/^ui\b/i, docs);
    expect(within(ui).getByText("Storefront design")).toBeTruthy();
    expect(within(ui).queryByText("Hero PRD")).toBeNull();

    expect(
      within(docs).queryByRole("region", { name: /^architecture\b/i }) ??
        within(docs).queryByRole("group", { name: /^architecture\b/i }),
      "a pillar without docs has no group",
    ).toBeNull();
  });

  test("the home updates live as the project's events come in", async () => {
    const api = fakeApi(seededHome());
    renderAt(HOME_PATH);
    const waiting = await section(WAITING);
    await within(waiting).findByText("The hero copy is shorter now");

    await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
    const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
    socket.open();
    await waitFor(() =>
      expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })),
    );
    socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 10 });

    // The human resolves the addressed comment; an agent asks a new question.
    const asked = question(api.home.threads[0].id, "Is the product shot ready for dark mode?");
    api.home = {
      ...api.home,
      waiting: { ...api.home.waiting, comments: [], questions: [...api.home.waiting.questions, asked] },
    };
    socket.receive({
      type: "event",
      seq: 11,
      event: "comment.status_changed",
      project_id: PROJECT_ID,
      at: AT,
      data: { comment_id: "x", doc_id: "y", from_status: "addressed", status: "resolved" },
    });
    await waitFor(() => {
      expect(within(waiting).queryByText("The hero copy is shorter now")).toBeNull();
      expect(within(waiting).getByText("Is the product shot ready for dark mode?")).toBeTruthy();
    });
  });
});
