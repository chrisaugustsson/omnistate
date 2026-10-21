// @vitest-environment jsdom
// US-041: the thread view, where the reviewer follows a thread live and
// decides on it. It is /w/<workspace id>/projects/<project id>/threads/<thread
// id>, and the project home links each thread's question there. The app is
// rendered with its own router on a memory history, the browser's fetch is
// replaced by a fake of the API, and the browser's WebSocket by a fake socket
// that the test drives.
//
// The page loads GET /api/v1/w/<workspace>/projects/<project>/threads/<thread>
// (crates/omni-api/src/threads.rs), which answers {thread} with its question,
// criteria, phases, every post (the human reads sealed phases too) and its
// current decision, a Decision body or null. The page shows:
//
// - the thread's question as a heading;
// - one section (or group) per phase, named "Phase <number>", holding the
//   posts of that phase; a sealed phase says so ("sealed") in its name or
//   inside it, an open phase does not. A post is a list item or an article
//   that shows its summary and, unless the reader asked for summaries only,
//   its body (`body_html`, which the API has already made safe);
// - a toggle named for summaries ("Summaries only", a switch, checkbox, radio
//   or pressed button) that switches between full posts and summaries only;
// - a form to post: a "Type" choice (a select whose option values are
//   `directive`, `note` and `question`, or radios named so), a "Summary" field,
//   a "Body" field and a "Post" button. Each post has a "Reply" button; a post
//   written after it links to that post. A directive is sent to POST
//   .../threads/<thread>/directives {summary, body} (or to .../posts with type
//   `directive`), a note or question to POST .../threads/<thread>/posts {type,
//   summary, body, links}, a reply carrying {"kind": "post", "post_id"} in
//   `links`;
// - a section named "Decision" with every field of the current decision: its
//   status, direction, chosen option, author, criteria, one row (list item or
//   table row) per option with its scores and why it lost, the assumptions,
//   the open questions, the dissent and the posts it relies on (by their
//   summaries). While it is proposed it has "Approve" and "Reject" buttons:
//   Approve calls POST .../decisions/<id>/approve, and Reject needs a reason
//   (a field named "Reason") before it calls POST .../decisions/<id>/reject
//   {reason}.
//
// The page subscribes to the project's live events and loads the thread again
// when one comes, so posts, phases and decisions appear without a reload.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const CHANGESET_ID = "0192d7a0-2222-7000-8000-000000000002";
const THREAD_ID = "0192d7a0-2222-7000-8000-000000000003";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const THREAD_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/threads/${THREAD_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T10:00:00Z";
const QUESTION = "Which hero layout?";
const CRITERIA = ["Readable at 320 px", "Fast to build"];
const HUMAN = { kind: "human", user_id: USER.id };

function agent(role: string) {
  return { kind: "agent", run: "run-1", role, model: "test-model", harness: "test-harness" };
}

let counter = 0;
/** A UUIDv7-shaped ID that no other fixture has. */
function newId() {
  counter += 1;
  return `0192d7a0-4444-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

type Post = ReturnType<typeof post>;

function post(
  phase: number,
  type: string,
  author: Record<string, unknown>,
  summary: string,
  body: string,
  links: Record<string, unknown>[] = [],
) {
  return {
    id: newId(),
    thread_id: THREAD_ID,
    phase,
    type,
    author,
    summary,
    body,
    body_html: `<p>${body}</p>`,
    links,
    attachments: [] as string[],
    confidence: null as number | null,
    replaces: null as string | null,
    created_at: AT,
  };
}

function phase(number: number, sealed: boolean, posts: Post[]) {
  const posters = new Map<string, { run: string | null; role: string | null }>();
  for (const p of posts) {
    if (p.author.kind !== "agent") continue;
    const run = (p.author.run as string) ?? null;
    const role = (p.author.role as string) ?? null;
    posters.set(`${run}/${role}`, { run, role });
  }
  return { number, sealed, started_at: AT, posted_by: [...posters.values()] };
}

type Decision = ReturnType<typeof decision>;

function decision(direction: string, reliesOn: string[], status = "proposed") {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    thread_id: THREAD_ID,
    changeset_id: CHANGESET_ID,
    status,
    author: agent("moderator"),
    direction,
    chosen: "Split hero",
    criteria: CRITERIA,
    options: [
      {
        name: "Split hero",
        scores: { "Readable at 320 px": 9, "Fast to build": 6 },
        lost_because: null as string | null,
      },
      {
        name: "Carousel",
        scores: { "Readable at 320 px": 3, "Fast to build": 7 },
        lost_because: "Only the first slide is seen on phones",
      },
    ],
    assumptions: ["Most visitors arrive on phones"],
    open_questions: ["Does the hero need a video variant?"],
    dissent: [{ by: "run-1/critic", text: "The split hero crops the product shot" }],
    posts: reliesOn,
    rejection_reason: null as string | null,
    created_at: AT,
  };
}

/**
 * A thread with an open first phase and a sealed second one, two agents
 * posting in each, and a proposed decision that relies on two of the posts.
 */
function seededThread() {
  const finding = post(
    1,
    "finding",
    agent("critic"),
    "Most visits come from phones",
    "Analytics show seven of ten visits from phones.",
  );
  const position = post(
    1,
    "position",
    agent("designer"),
    "The split hero reads best on phones",
    "Split keeps the copy beside the product shot.",
  );
  const objection = post(
    2,
    "objection",
    agent("critic"),
    "The carousel hides the offer",
    "Only the first slide is ever looked at.",
  );
  const support = post(
    2,
    "support",
    agent("designer"),
    "Backing the split hero",
    "It matches the brand guide.",
  );
  const posts = [finding, position, objection, support];
  const current = decision("Use the split hero with the copy on the left", [finding.id, objection.id]);
  return {
    id: THREAD_ID,
    project_id: PROJECT_ID,
    changeset_id: CHANGESET_ID,
    question: QUESTION,
    criteria: CRITERIA,
    run_tag: "run-1",
    created_at: AT,
    phases: [phase(1, false, [finding, position]), phase(2, true, [objection, support])],
    decision: current as Decision | null,
    posts,
  };
}

type Thread = ReturnType<typeof seededThread>;
type Call = { method: string; path: string; body: Record<string, unknown> | null };

/** A fake of the API: the auth routes, the project's home and the thread, which the test may change, and the calls it got. */
function fakeApi(thread: Thread) {
  const api = { thread, threadLoads: 0, calls: [] as Call[] };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  const decisionPath = (id: string, action: string) => `${API}/decisions/${id}/${action}`;

  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url, "http://localhost:3000").pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    let text: string | null = null;
    if (typeof init?.body === "string") text = init.body;
    else if (input instanceof Request) text = await input.clone().text();
    let body: Record<string, unknown> | null;
    try {
      body = text ? JSON.parse(text) : null;
    } catch {
      body = null;
    }
    api.calls.push({ method, path, body });

    const current = api.thread.decision;
    switch (`${method} ${path}`) {
      case "GET /api/v1/auth/me":
        return json(200, { user: USER });
      case "GET /api/v1/auth/workspaces":
        return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
      case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
        return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
      case `GET ${API}/home`: {
        const listed = { ...api.thread, decision: null };
        delete (listed as Partial<Thread>).posts;
        return json(200, {
          project_id: PROJECT_ID,
          docs: [],
          changesets: [],
          threads: [listed],
          waiting: { decisions: [], changesets: [], comments: [], questions: [] },
        });
      }
      case `GET ${API}/threads/${THREAD_ID}`:
        api.threadLoads += 1;
        return json(200, { thread: api.thread });
      case `POST ${API}/threads/${THREAD_ID}/posts`:
      case `POST ${API}/threads/${THREAD_ID}/directives`: {
        const sent = body ?? {};
        const type = path.endsWith("/directives") ? "directive" : String(sent.type);
        const phases = api.thread.phases;
        const created = post(
          phases[phases.length - 1].number,
          type,
          HUMAN,
          String(sent.summary),
          String(sent.body),
          (sent.links as Record<string, unknown>[] | undefined) ?? [],
        );
        api.thread = { ...api.thread, posts: [...api.thread.posts, created] };
        return json(201, { post: created });
      }
      default:
        break;
    }
    if (current && method === "POST" && path === decisionPath(current.id, "approve")) {
      api.thread = { ...api.thread, decision: { ...current, status: "approved" } };
      return json(200, { decision: api.thread.decision });
    }
    if (current && method === "POST" && path === decisionPath(current.id, "reject")) {
      const reason = typeof body?.reason === "string" ? body.reason.trim() : "";
      if (!reason) return json(400, { error: "invalid_request" });
      api.thread = {
        ...api.thread,
        decision: { ...current, status: "rejected", rejection_reason: reason },
      };
      return json(200, { decision: api.thread.decision });
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

/** The page's section (or group) named `name` as it is now. */
function sectionNow(name: RegExp) {
  const found = screen.queryByRole("region", { name }) ?? screen.queryByRole("group", { name });
  expect(found, `a section named ${name}`).toBeTruthy();
  return found as HTMLElement;
}

const ITEM = 'li, tr, article, [role="listitem"], [role="row"], [role="article"]';

/** The post (a list item or article) of `container` whose summary is `summary`. */
function postShowing(container: HTMLElement, summary: string) {
  const element = within(container).getByText(summary);
  const item = element.closest(ITEM);
  expect(item, `the post "${summary}" is a list item or an article`).toBeTruthy();
  return item as HTMLElement;
}

/** Every list item or table row of `container` that shows `text`. */
function rowsShowing(container: HTMLElement, text: string) {
  return within(container)
    .getAllByText(text)
    .map((element) => element.closest(ITEM))
    .filter((row): row is HTMLElement => row !== null);
}

const PHASE_1 = /^phase 1\b/i;
const PHASE_2 = /^phase 2\b/i;
const PHASE_3 = /^phase 3\b/i;
const DECISION = /^(current )?decision\b/i;

/** Whether a phase's section says it is sealed, in its name or in a text inside it. */
function saysSealed(phaseSection: HTMLElement) {
  const sealed = /\bsealed\b/i;
  const labelledBy = phaseSection.getAttribute("aria-labelledby");
  const label = labelledBy ? (document.getElementById(labelledBy)?.textContent ?? "") : "";
  return (
    sealed.test(phaseSection.getAttribute("aria-label") ?? "") ||
    sealed.test(label) ||
    within(phaseSection).queryAllByText(sealed).length > 0
  );
}

/** Whether an element of `container` (or it) shows the word, in its own text. */
function showsWord(container: HTMLElement, word: RegExp) {
  return within(container).queryAllByText(word).length > 0;
}

/** Whether an element of `container` (or it) shows the number `n` in its own text, not as a part of another number. */
function showsNumber(container: HTMLElement, n: number) {
  const number = new RegExp(`(^|[^\\d.])${n}([^\\d.]|$)`);
  return [container, ...Array.from(container.querySelectorAll("*"))].some((element) =>
    number.test(
      Array.from(element.childNodes)
        .filter((node) => node.nodeType === Node.TEXT_NODE)
        .map((node) => node.textContent)
        .join(""),
    ),
  );
}

/** The control that switches the posts to summaries only. */
function summariesToggle() {
  for (const role of ["switch", "checkbox", "radio", "button"] as const) {
    const found = screen.queryByRole(role, { name: /summar/i });
    if (found) return found;
  }
  throw new Error("no toggle named for summaries");
}

/** Chooses the type of the post to write: a radio named so, or the "Type" select. */
function chooseType(type: "directive" | "note" | "question") {
  const radio = screen.queryByRole("radio", { name: new RegExp(`^${type}\\b`, "i") });
  if (radio) {
    fireEvent.click(radio);
    return;
  }
  const select = screen.getByRole("combobox", { name: /^(post )?type\b/i });
  fireEvent.change(select, { target: { value: type } });
  expect((select as HTMLSelectElement).value).toBe(type);
}

/** Fills in the post form and sends it. */
function writePost(summary: string, body: string) {
  fireEvent.change(screen.getByRole("textbox", { name: /^summary\b/i }), {
    target: { value: summary },
  });
  fireEvent.change(screen.getByRole("textbox", { name: /^(body|text|message|markdown)\b/i }), {
    target: { value: body },
  });
  fireEvent.click(screen.getByRole("button", { name: /^(post|send)\b/i }));
}

/** The calls the page made to `path` with `method`. */
function callsTo(api: ReturnType<typeof fakeApi>, method: string, path: string) {
  return api.calls.filter((call) => call.method === method && call.path === path);
}

/** Opens the live socket and confirms the page's subscription to the project. */
async function openSocket() {
  await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
  const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
  socket.open();
  await waitFor(() =>
    expect(socket.sent).toContainEqual(
      expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID }),
    ),
  );
  socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 20 });
  return socket;
}

describe("the thread view", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  test("the project home links each thread to its view", async () => {
    fakeApi(seededThread());
    const router = renderAt(`/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`);
    const link = await screen.findByRole("link", { name: QUESTION });
    expect(link.getAttribute("href")).toBe(THREAD_PATH);
    fireEvent.click(link);
    await waitFor(() => expect(router.state.location.pathname).toBe(THREAD_PATH));
    await section(PHASE_1);
  });

  test("the posts are grouped by phase and a sealed phase is labeled but shows every post", async () => {
    const thread = seededThread();
    fakeApi(thread);
    renderAt(THREAD_PATH);
    expect(await screen.findByRole("heading", { name: QUESTION })).toBeTruthy();

    const first = await section(PHASE_1);
    const second = await section(PHASE_2);
    const [finding, position, objection, support] = thread.posts;

    expect(postShowing(first, finding.summary)).toBeTruthy();
    expect(postShowing(first, position.summary)).toBeTruthy();
    expect(within(first).queryByText(objection.summary)).toBeNull();
    expect(within(first).queryByText(support.summary)).toBeNull();

    // The human reads the sealed phase whole: both agents' posts, with their text.
    expect(postShowing(second, objection.summary).textContent).toContain(objection.body);
    expect(postShowing(second, support.summary).textContent).toContain(support.body);
    expect(within(second).queryByText(finding.summary)).toBeNull();

    expect(saysSealed(second), "the sealed phase is labeled as sealed").toBe(true);
    expect(saysSealed(first), "the open phase is not labeled as sealed").toBe(false);
  });

  test("new posts, phases and decisions appear live", async () => {
    const api = fakeApi(seededThread());
    renderAt(THREAD_PATH);
    await section(PHASE_2);
    const socket = await openSocket();

    // An agent posts in the sealed phase.
    const before = api.threadLoads;
    const late = post(2, "position", agent("analyst"), "Phones need the offer above the fold", "Keep the price in view.");
    api.thread = {
      ...api.thread,
      phases: [api.thread.phases[0], phase(2, true, [...api.thread.posts.slice(2), late])],
      posts: [...api.thread.posts, late],
    };
    socket.receive({
      type: "event",
      seq: 21,
      event: "post.created",
      project_id: PROJECT_ID,
      at: AT,
      data: { post_id: late.id, thread_id: THREAD_ID, changeset_id: CHANGESET_ID, phase: 2, type: "position" },
    });
    await waitFor(() => expect(within(sectionNow(PHASE_2)).getByText(late.summary)).toBeTruthy());
    expect(api.threadLoads).toBeGreaterThan(before);

    // An agent starts a third phase and posts in it.
    const opening = post(3, "note", agent("critic"), "Third round: the footer", "The footer needs the same care.");
    api.thread = {
      ...api.thread,
      phases: [...api.thread.phases, phase(3, false, [opening])],
      posts: [...api.thread.posts, opening],
    };
    socket.receive({
      type: "event",
      seq: 22,
      event: "phase.started",
      project_id: PROJECT_ID,
      at: AT,
      data: { thread_id: THREAD_ID, number: 3, sealed: false },
    });
    await section(PHASE_3);
    await waitFor(() => expect(within(sectionNow(PHASE_3)).getByText(opening.summary)).toBeTruthy());

    // A new proposal supersedes the current decision.
    const newer = decision("Use the split hero with a sticky price", [late.id]);
    api.thread = { ...api.thread, decision: newer };
    socket.receive({
      type: "event",
      seq: 23,
      event: "decision.proposed",
      project_id: PROJECT_ID,
      at: AT,
      data: { decision_id: newer.id, thread_id: THREAD_ID, changeset_id: CHANGESET_ID, status: "proposed" },
    });
    await waitFor(() => {
      const shown = sectionNow(DECISION);
      expect(within(shown).getByText(newer.direction)).toBeTruthy();
      expect(within(shown).queryByText("Use the split hero with the copy on the left")).toBeNull();
    });
  });

  test("a toggle switches between full posts and summaries only", async () => {
    const thread = seededThread();
    fakeApi(thread);
    renderAt(THREAD_PATH);
    await section(PHASE_2);
    const phaseOf = (p: Post) => sectionNow(p.phase === 1 ? PHASE_1 : PHASE_2);
    for (const p of thread.posts) {
      expect(postShowing(phaseOf(p), p.summary).textContent).toContain(p.body);
    }

    const toggle = summariesToggle();
    fireEvent.click(toggle);
    await waitFor(() => {
      for (const p of thread.posts) {
        const item = postShowing(phaseOf(p), p.summary);
        expect(item.textContent, "summaries only hide the body").not.toContain(p.body);
        expect(screen.queryByText(p.body), "summaries only hide the body").toBeNull();
      }
    });
    // The phases stay.
    await section(PHASE_1);
    await section(PHASE_2);

    const full = screen.queryByRole("radio", { name: /full/i });
    fireEvent.click(full ?? summariesToggle());
    await waitFor(() => {
      for (const p of thread.posts) {
        expect(postShowing(phaseOf(p), p.summary).textContent).toContain(p.body);
      }
    });
  });

  test("the reviewer posts a directive", async () => {
    const api = fakeApi(seededThread());
    renderAt(THREAD_PATH);
    await section(PHASE_1);

    chooseType("directive");
    writePost("Keep the hero under 600 px tall", "Phones must see the offer without scrolling.");

    await waitFor(() => {
      const viaDirectives = callsTo(api, "POST", `${API}/threads/${THREAD_ID}/directives`);
      const viaPosts = callsTo(api, "POST", `${API}/threads/${THREAD_ID}/posts`).filter(
        (call) => call.body?.type === "directive",
      );
      const sent = [...viaDirectives, ...viaPosts];
      expect(sent).toHaveLength(1);
      expect(sent[0].body).toMatchObject({
        summary: "Keep the hero under 600 px tall",
        body: "Phones must see the offer without scrolling.",
      });
    });
  });

  test("the reviewer posts a note and a question", async () => {
    const api = fakeApi(seededThread());
    renderAt(THREAD_PATH);
    await section(PHASE_1);
    const posts = () => callsTo(api, "POST", `${API}/threads/${THREAD_ID}/posts`);

    chooseType("note");
    writePost("The brand guide is being revised", "Expect new colors next month.");
    await waitFor(() => expect(posts()).toHaveLength(1));
    expect(posts()[0].body).toMatchObject({
      type: "note",
      summary: "The brand guide is being revised",
      body: "Expect new colors next month.",
    });
    expect(posts()[0].body?.links ?? [], "a post that replies to nothing links to nothing").toEqual([]);

    chooseType("question");
    writePost("Did anyone test on a small Android phone?", "The 320 px criterion came from one.");
    await waitFor(() => expect(posts()).toHaveLength(2));
    expect(posts()[1].body).toMatchObject({
      type: "question",
      summary: "Did anyone test on a small Android phone?",
      body: "The 320 px criterion came from one.",
    });
  });

  test("a reply links to the post it answers", async () => {
    const thread = seededThread();
    const api = fakeApi(thread);
    renderAt(THREAD_PATH);
    const second = await section(PHASE_2);
    const [, , objection] = thread.posts;

    const item = postShowing(second, objection.summary);
    fireEvent.click(within(item).getByRole("button", { name: /^reply\b/i }));
    chooseType("question");
    writePost("Which slide did the analytics count?", "Was it the first one only?");

    const posts = () => callsTo(api, "POST", `${API}/threads/${THREAD_ID}/posts`);
    await waitFor(() => expect(posts()).toHaveLength(1));
    expect(posts()[0].body).toMatchObject({
      type: "question",
      summary: "Which slide did the analytics count?",
    });
    expect(posts()[0].body?.links).toContainEqual({ kind: "post", post_id: objection.id });
  });

  test("the current decision shows all of its fields", async () => {
    const thread = seededThread();
    fakeApi(thread);
    renderAt(THREAD_PATH);
    const shown = await section(DECISION);
    const current = thread.decision as Decision;
    const [finding, , objection] = thread.posts;

    await waitFor(() => expect(within(shown).getByText(current.direction)).toBeTruthy());
    expect(showsWord(shown, /\bproposed\b/i), "the status").toBe(true);
    expect(shown.textContent, "the author's role").toContain("moderator");
    expect(within(shown).getAllByText(/\bSplit hero\b/).length, "the chosen option").toBeGreaterThan(0);
    for (const criterion of CRITERIA) {
      expect(within(shown).getAllByText(new RegExp(criterion)).length, criterion).toBeGreaterThan(0);
    }

    const split = rowsShowing(shown, "Split hero");
    expect(
      split.some((row) => showsNumber(row, 9) && showsNumber(row, 6)),
      "the chosen option's row has its scores",
    ).toBe(true);
    const carousel = rowsShowing(shown, "Carousel");
    expect(
      carousel.some(
        (row) =>
          showsNumber(row, 3) &&
          showsNumber(row, 7) &&
          (row.textContent ?? "").includes("Only the first slide is seen on phones"),
      ),
      "the losing option's row has its scores and why it lost",
    ).toBe(true);

    expect(within(shown).getByText(/Most visitors arrive on phones/), "the assumptions").toBeTruthy();
    expect(within(shown).getByText(/Does the hero need a video variant\?/), "the open questions").toBeTruthy();
    expect(shown.textContent, "who dissents").toContain("run-1/critic");
    expect(within(shown).getByText(/The split hero crops the product shot/), "the dissent").toBeTruthy();
    expect(within(shown).getByText(new RegExp(finding.summary)), "a post it relies on").toBeTruthy();
    expect(within(shown).getByText(new RegExp(objection.summary)), "a post it relies on").toBeTruthy();
  });

  test("approving the proposed decision calls the API and shows it approved", async () => {
    const thread = seededThread();
    const api = fakeApi(thread);
    renderAt(THREAD_PATH);
    const shown = await section(DECISION);
    const current = thread.decision as Decision;

    const approve = await within(shown).findByRole("button", { name: /^approve\b/i });
    expect(within(shown).getByRole("button", { name: /^reject\b/i })).toBeTruthy();
    fireEvent.click(approve);

    await waitFor(() => expect(callsTo(api, "POST", `${API}/decisions/${current.id}/approve`)).toHaveLength(1));
    await waitFor(() => {
      const now = sectionNow(DECISION);
      expect(showsWord(now, /\bapproved\b/i), "the status").toBe(true);
      expect(within(now).queryByRole("button", { name: /^approve\b/i }), "an approved decision is not approved again").toBeNull();
      expect(within(now).queryByRole("button", { name: /^reject\b/i })).toBeNull();
    });
  });

  test("rejecting the proposed decision requires a reason", async () => {
    const thread = seededThread();
    const api = fakeApi(thread);
    renderAt(THREAD_PATH);
    const shown = await section(DECISION);
    const current = thread.decision as Decision;
    const rejects = () => callsTo(api, "POST", `${API}/decisions/${current.id}/reject`);

    fireEvent.click(await within(shown).findByRole("button", { name: /^reject\b/i }));
    const reason = await within(shown).findByRole("textbox", { name: /reason/i });

    // Without a reason nothing is sent.
    const confirm = () => {
      const buttons = within(shown).getAllByRole("button", { name: /^reject\b/i });
      return buttons[buttons.length - 1];
    };
    fireEvent.click(confirm());
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(rejects(), "a rejection without a reason is not sent").toHaveLength(0);

    fireEvent.change(reason, { target: { value: "The product shot must stay whole" } });
    fireEvent.click(confirm());
    await waitFor(() => expect(rejects()).toHaveLength(1));
    expect(rejects()[0].body).toEqual({ reason: "The product shot must stay whole" });

    await waitFor(() => {
      const now = sectionNow(DECISION);
      expect(showsWord(now, /\brejected\b/i), "the status").toBe(true);
      expect(now.textContent, "the rejection's reason").toContain("The product shot must stay whole");
    });
  });

  test("a decision that is no longer proposed has no Approve or Reject button", async () => {
    const thread = seededThread();
    const current = thread.decision as Decision;
    thread.decision = { ...current, status: "rejected", rejection_reason: "Too risky for launch" };
    fakeApi(thread);
    renderAt(THREAD_PATH);
    const shown = await section(DECISION);

    await waitFor(() => expect(within(shown).getByText(current.direction)).toBeTruthy());
    expect(showsWord(shown, /\brejected\b/i), "the status").toBe(true);
    expect(shown.textContent, "the rejection's reason").toContain("Too risky for launch");
    expect(within(shown).queryByRole("button", { name: /^approve\b/i })).toBeNull();
    expect(within(shown).queryByRole("button", { name: /^reject\b/i })).toBeNull();
  });
});
