// @vitest-environment jsdom
// US-042: the doc viewer, where the reviewer reads an HTML doc and comments
// on its blocks. It is /w/<workspace id>/projects/<project id>/docs/<doc id>,
// and the project home links each doc's title there. The app is rendered with
// its own router on a memory history, the browser's fetch is replaced by a
// fake of the API, and the browser's WebSocket by a fake socket that the test
// drives.
//
// The doc is shown in an `<iframe sandbox="allow-scripts">` (the only token:
// never `allow-same-origin`) whose `srcdoc` is the viewer's own document
// (src/viewer/document.ts, `viewerDocument({ workspaceId, origin })` with the
// page's origin), so its content security policy is the one that
// src/testing/viewer-policy.ts checks. jsdom does not run a `srcdoc`, so the
// test plays the frame: it sends the page the frame's messages, as a
// MessageEvent on the page's window whose `source` is the frame's window, and
// it records what the page posts to the frame's window. The messages are those
// of web/src/doc-viewer-frame.test.ts, which tests the frame itself in Chrome:
//
// - frame to page: `{ type: "viewer.ready" }`, and `{ type: "viewer.select",
//   node }` when the reviewer clicks a block. The page ignores messages whose
//   `source` is not its frame.
// - page to frame: `{ type: "viewer.show", html, pins, selected, claims }`
//   after each `viewer.ready` and whenever any of it changes, where `html` is
//   the HTML of what is shown, `pins` is `[{ node, count }]` (the blocks of
//   the shown HTML that have comments, and how many) and `selected` is the
//   selected block's node ID or `null`.
//
// The page loads the doc (GET .../docs/<doc>), the HTML of its head (GET
// .../docs/<doc>/html, US-112: there is no version picker and no draft view)
// and the doc's comments (GET .../comments?doc=<doc>). It shows:
//
// - the doc's title as a heading;
// - a section named "Comments" (a heading names it) with the comments of the
//   selected block and, once a block is selected, a field named "Comment" and
//   a button "Comment" (or "Add comment", or "Post") that sends POST
//   .../docs/<doc>/comments {node, text};
// - the head as agents write it: the page subscribes to the project's live
//   events and reads the head again on `doc.changed`, keeping the selected
//   block while it is still there. `?node=<block>` opens the doc with that
//   block selected, and the frame is told to scroll it into view (`reveal`).
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { viewerPolicyProblems } from "./testing/viewer-policy";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const DOC_ID = "0192d7a0-5555-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const DOC_TITLE = "Checkout PRD";
const DOC_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/docs/${DOC_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T10:00:00Z";
const HUMAN = { kind: "human", user_id: USER.id };
const WRITER = { run: "run-1", role: "writer", model: "test-model", harness: "test-harness" };

const HEAD_HTML = `<section data-omni-id="s0000001">
<h1 data-omni-id="h0000001">Checkout</h1>
<p data-omni-id="p0000001">Orders ship in one day.</p>
<p data-omni-id="p0000002">Refunds take a week.</p>
</section>`;

let counter = 0;
function newId() {
  counter += 1;
  return `0192d7a0-6666-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

type Comment = ReturnType<typeof comment>;

function comment(node: string, text: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    doc_id: DOC_ID,
    node,
    author: HUMAN,
    text,
    replies: [] as unknown[],
    orphaned: false,
    waiting_on_you: false,
    created_at: AT,
    snapshot: undefined as string | undefined,
  };
}

/** A reply to a comment, by an agent of `role`. */
function agentReply(role: string, text: string, at: string) {
  return { id: newId(), author: { ...WRITER, kind: "agent", role }, text, created_at: at };
}

/** A comment on the block `node`, which an agent removed: its snapshot is the block's HTML. */
function orphan(node: string, text: string, snapshot: string) {
  return { ...comment(node, text), orphaned: true, snapshot };
}

/** The doc's two comments, one on each paragraph. */
function seededComments() {
  return [comment("p0000001", "Is one day right?"), comment("p0000002", "Say which week.")];
}

type Call = { method: string; path: string; search: URLSearchParams; body: Record<string, unknown> | null };

/** A fake of the API: the auth routes, the project, the doc with its head and comments, and the calls it got. */
function fakeApi({ kind = "html", head = HEAD_HTML } = {}) {
  const api = {
    comments: seededComments() as Comment[],
    head,
    deleted: false,
    calls: [] as Call[],
  };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  const doc = {
    id: DOC_ID,
    project_id: PROJECT_ID,
    kind,
    pillar: "spec",
    title: DOC_TITLE,
    created_at: AT,
    drafts: [],
    claims: [],
  };

  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(raw, "http://localhost:3000");
    const path = url.pathname;
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
    api.calls.push({ method, path, search: url.searchParams, body });
    if (api.deleted && path.startsWith(`${API}/docs/${DOC_ID}`)) return json(404, { error: "not_found" });

    switch (`${method} ${path}`) {
      case "GET /api/v1/auth/me":
        return json(200, { user: USER });
      case "GET /api/v1/auth/workspaces":
        return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
      case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
        return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
      case `GET ${API}/home`:
        return json(200, {
          project_id: PROJECT_ID,
          docs: api.deleted ? [] : [doc],
          threads: [],
          waiting: { comments: [] },
        });
      case `GET ${API}/docs/${DOC_ID}`:
        return json(200, { doc });
      case `GET ${API}/docs/${DOC_ID}/html`:
        return json(200, { doc_id: DOC_ID, html: api.head });
      case `GET ${API}/comments`: {
        const docFilter = url.searchParams.get("doc");
        const listed = api.comments.filter((c) => !docFilter || c.doc_id === docFilter);
        return json(200, { project_id: PROJECT_ID, comments: listed });
      }
      case `POST ${API}/docs/${DOC_ID}/comments`: {
        const node = typeof body?.node === "string" ? body.node : "";
        const said = typeof body?.text === "string" ? body.text.trim() : "";
        if (!node || !said) return json(400, { error: "invalid_request" });
        const created = comment(node, said);
        api.comments = [...api.comments, created];
        return json(201, { comment: created });
      }
      default:
        break;
    }
    const replied = api.comments.find((c) => path === `${API}/comments/${c.id}/replies`);
    if (method === "POST" && replied) {
      const reply = { id: newId(), author: HUMAN, text: String(body?.text ?? ""), created_at: new Date().toISOString() };
      replied.replies = [...replied.replies, reply];
      replied.waiting_on_you = false;
      return json(201, { comment: replied });
    }
    const deleted = api.comments.find((c) => path === `${API}/comments/${c.id}`);
    if (method === "DELETE" && deleted) {
      api.comments = api.comments.filter((c) => c !== deleted);
      return new Response(null, { status: 204 });
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

// The frame ----------------------------------------------------------------------------

type Show = {
  type: "viewer.show";
  html: string;
  pins: { node: string; count: number }[];
  selected: string | null;
  claims?: unknown;
  reveal?: string;
};

/** What the page posted to each frame window, recorded from the moment the frame exists. */
const posted = new Map<Window, unknown[]>();
let observer: MutationObserver | null = null;

function watchFrames() {
  const attach = () => {
    for (const frame of Array.from(document.querySelectorAll("iframe"))) {
      const target = frame.contentWindow;
      if (!target || posted.has(target)) continue;
      const list: unknown[] = [];
      posted.set(target, list);
      target.addEventListener("message", (event) => list.push((event as MessageEvent).data));
    }
  };
  observer = new MutationObserver(attach);
  observer.observe(document, { subtree: true, childList: true, attributes: true });
}

/** The doc's frame, once the page shows it. */
async function findFrame() {
  return waitFor(() => {
    const frames = document.querySelectorAll("iframe");
    expect(frames.length, "the page shows the doc in one frame").toBe(1);
    return frames[0] as HTMLIFrameElement;
  });
}

function currentFrameWindow() {
  const frame = document.querySelector("iframe");
  expect(frame, "the doc's frame").toBeTruthy();
  expect(frame!.contentWindow, "the frame has a window").toBeTruthy();
  return frame!.contentWindow!;
}

/** Sends the page a message as the frame (or, with `source`, as another window). */
function fromFrame(data: unknown, source: Window = currentFrameWindow()) {
  window.dispatchEvent(new MessageEvent("message", { data, origin: "null", source }));
}

/** The `viewer.show` messages the page posted to the current frame. */
function shows(): Show[] {
  return (posted.get(currentFrameWindow()) ?? []).filter(
    (message): message is Show => (message as { type?: string } | null)?.type === "viewer.show",
  );
}

/**
 * Waits until the latest `viewer.show` the page posted to the frame passes
 * `check`. Until the page has shown the frame anything, the test says
 * `viewer.ready` again, as a frame that loads does.
 */
async function shown(check: (show: Show) => void) {
  return waitFor(
    () => {
      const all = shows();
      if (!all.length) fromFrame({ type: "viewer.ready" });
      const latest = all[all.length - 1];
      expect(latest, "the page posted viewer.show to the frame").toBeTruthy();
      check(latest);
      return latest;
    },
    { timeout: 3_000 },
  );
}

/** The reviewer clicks the block `node` in the frame. */
async function select(node: string) {
  await shown(() => {});
  fromFrame({ type: "viewer.select", node });
}

// The page -----------------------------------------------------------------------------

/** The page's section (or group) named `name`. */
async function section(name: RegExp) {
  return waitFor(() => {
    const found = screen.queryByRole("region", { name }) ?? screen.queryByRole("group", { name });
    expect(found, `a section named ${name}`).toBeTruthy();
    return found as HTMLElement;
  });
}

const COMMENTS = /^comments\b/i;

/** Writes a comment in the comments section and sends it. */
async function writeComment(text: string) {
  const comments = await section(COMMENTS);
  const field = await within(comments).findByRole("textbox", { name: /comment/i });
  fireEvent.change(field, { target: { value: text } });
  fireEvent.click(within(comments).getByRole("button", { name: /^(add comment|comment|post)$/i }));
}

/** Opens the live socket and confirms the page's subscription to the project. */
async function openSocket() {
  await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
  const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
  socket.open();
  await waitFor(() =>
    expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })),
  );
  socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 40 });
  return socket;
}

function event(seq: number, name: string, data: Record<string, unknown>) {
  return { type: "event", seq, event: name, project_id: PROJECT_ID, at: AT, data };
}

function callsTo(api: ReturnType<typeof fakeApi>, method: string, path: string) {
  return api.calls.filter((call) => call.method === method && call.path === path);
}

describe("the doc viewer", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    FakeWebSocket.instances = [];
    vi.stubGlobal("WebSocket", FakeWebSocket);
    posted.clear();
    watchFrames();
  });

  afterEach(() => {
    observer?.disconnect();
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  test("a design system opens as browsable tokens, never a raw JSON document", async () => {
    const system = {
      $extensions: { omnistate: { modes: ["light", "dark"] } },
      color: {
        $type: "color",
        brand: { $value: "#fe6233", $description: "The brand color" },
        surface: {
          $value: "#ffffff",
          $extensions: { omnistate: { modes: { dark: "#171717" } } },
        },
        background: { $value: "{color.surface}" },
      },
      $components: [],
    };
    const api = fakeApi({ kind: "design-system", head: JSON.stringify(system) });
    renderAt(DOC_PATH);
    const catalog = await screen.findByRole("region", { name: "Design tokens" });
    expect(within(catalog).getByText("color.brand")).toBeTruthy();
    expect(within(catalog).getByText("#fe6233")).toBeTruthy();
    expect(document.querySelector("iframe"), "the token payload is never sent to the HTML frame").toBeNull();

    fireEvent.change(screen.getByRole("combobox", { name: "Mode" }), { target: { value: "dark" } });
    expect(within(catalog).getAllByText("#171717")).toHaveLength(2);
    fireEvent.change(screen.getByRole("searchbox", { name: "Search design system" }), {
      target: { value: "brand" },
    });
    expect(within(catalog).queryByText("color.surface")).toBeNull();

    const socket = await openSocket();
    api.head = JSON.stringify({ ...system, color: { ...system.color, brand: { $value: "#123456" } } });
    socket.receive(event(41, "doc.changed", { doc_id: DOC_ID, agent: WRITER }));
    expect(await within(catalog).findByText("#123456")).toBeTruthy();
  });

  test("the doc renders in a frame sandboxed to scripts alone, under the viewer's policy", async () => {
    fakeApi();
    renderAt(DOC_PATH);
    const frame = await findFrame();
    const tokens = (frame.getAttribute("sandbox") ?? "").trim().split(/\s+/);
    expect(tokens).toEqual(["allow-scripts"]);
    expect(frame.hasAttribute("src") && frame.getAttribute("src") !== "about:blank").toBe(false);
    const srcdoc = frame.getAttribute("srcdoc") ?? "";
    expect(srcdoc, "the frame's document is its srcdoc").not.toBe("");
    expect(viewerPolicyProblems(srcdoc, { workspaceId: WORKSPACE_ID, origin: window.location.origin })).toEqual([]);
  });

  test("the frame is shown the doc's head, and there is no version picker", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    expect(await screen.findByRole("heading", { name: DOC_TITLE })).toBeTruthy();
    await findFrame();
    const show = await shown((s) => expect(s.html).toBe(HEAD_HTML));
    expect(show.selected).toBeNull();
    expect(screen.queryByRole("combobox", { name: /version/i }), "no version picker").toBeNull();
    expect(
      callsTo(api, "GET", `${API}/docs/${DOC_ID}/html`).map((call) => call.search.get("version")),
      "the head is read, never a version",
    ).not.toContainEqual(expect.any(String));
  });

  test("selecting a block in the frame opens its comments, and the frame is told it is selected", async () => {
    fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    await select("p0000002");

    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText("Say which week.")).toBeTruthy());
    expect(within(comments).queryByText("Is one day right?")).toBeNull();
    await shown((s) => expect(s.selected).toBe("p0000002"));

    // A message from any other window is not the frame's.
    fromFrame({ type: "viewer.select", node: "p0000001" }, window);
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(within(comments).getByText("Say which week.")).toBeTruthy();
    expect(within(comments).queryByText("Is one day right?")).toBeNull();
    expect(shows()[shows().length - 1].selected).toBe("p0000002");
  });

  test("pins mark the blocks that have comments", async () => {
    fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    const show = await shown((s) => {
      expect(s.html).toBe(HEAD_HTML);
      expect(s.pins).toContainEqual({ node: "p0000002", count: 1 });
    });
    expect(show.pins.filter((pin) => pin.node === "h0000001" && pin.count > 0)).toEqual([]);
    expect(show.pins.filter((pin) => pin.node === "s0000001" && pin.count > 0)).toEqual([]);
  });

  test("a new comment attaches to the selected block of the doc", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    await select("p0000002");
    await writeComment("Name the week in days.");
    await waitFor(() =>
      expect(callsTo(api, "POST", `${API}/docs/${DOC_ID}/comments`).map((call) => call.body)).toEqual([
        { node: "p0000002", text: "Name the week in days." },
      ]),
    );
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText("Name the week in days.")).toBeTruthy());
    await shown((s) => expect(s.pins).toContainEqual({ node: "p0000002", count: 2 }));
  });

  test("when an agent changes the doc, the frame is shown the new head and the selected block stays selected", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    const frame = await findFrame();
    const socket = await openSocket();
    await select("p0000002");
    await shown((s) => expect(s.selected).toBe("p0000002"));

    api.head = HEAD_HTML.replace("Refunds take a week.", "Refunds take five days.");
    socket.receive(event(41, "doc.changed", { doc_id: DOC_ID, agent: WRITER }));
    const show = await shown((s) => expect(s.html).toContain("Refunds take five days."));
    expect(show.selected, "the selection is kept").toBe("p0000002");
    expect(document.querySelector("iframe"), "the same frame draws the new head, so it keeps its scroll").toBe(frame);
  });

  test("when an agent removes the selected block, the selection is cleared", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    const socket = await openSocket();
    await select("p0000002");
    await shown((s) => expect(s.selected).toBe("p0000002"));

    api.head = HEAD_HTML.replace('<p data-omni-id="p0000002">Refunds take a week.</p>\n', "");
    socket.receive(event(41, "doc.changed", { doc_id: DOC_ID, agent: WRITER }));
    await shown((s) => {
      expect(s.html).not.toContain("p0000002");
      expect(s.selected, "nothing is selected").toBeNull();
    });
    expect(await within(await section(COMMENTS)).findByText(/select a block/i)).toBeTruthy();
  });

  test("a link with ?node opens the doc with that block selected, and the frame scrolls it into view", async () => {
    fakeApi();
    renderAt(`${DOC_PATH}?node=p0000002`);
    await findFrame();
    const show = await shown((s) => expect(s.html).toBe(HEAD_HTML));
    expect(show.selected).toBe("p0000002");
    expect(show.reveal, "the frame is told to scroll the block into view").toBe("p0000002");
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText("Say which week.")).toBeTruthy());
  });

  test("selecting a block puts it in the address bar, in place of the entry", async () => {
    fakeApi();
    const router = renderAt(DOC_PATH);
    await findFrame();
    const entries = router.history.length;
    await select("p0000001");
    await waitFor(() => expect(router.state.location.search).toEqual({ node: "p0000001" }));
    expect(router.history.length, "the entry is replaced, not added").toBe(entries);
  });

  test("a comment shows its replies oldest first with who wrote them and when, and a reply of yours comes last", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-21T15:00:00Z"));
    const api = fakeApi();
    const asked = api.comments[1];
    asked.replies = [
      agentReply("writer", "Changed it to seven days.", "2026-10-21T12:00:00Z"),
      agentReply("editor", "Linked the refund policy too.", "2026-10-21T14:00:00Z"),
    ];
    renderAt(DOC_PATH);
    await findFrame();
    await select("p0000002");
    const comments = await section(COMMENTS);
    const card = (await within(comments).findByText("Say which week.")).closest("li") as HTMLElement;
    const replies = () => within(card).getAllByRole("listitem").map((item) => item.textContent);
    expect(replies()).toEqual([
      expect.stringMatching(/writer\s*3 h ago\s*Changed it to seven days\.$/),
      expect.stringMatching(/editor\s*1 h ago\s*Linked the refund policy too\.$/),
    ]);

    fireEvent.change(within(card).getByRole("textbox", { name: /^reply$/i }), { target: { value: "Thanks, that reads well." } });
    fireEvent.click(within(card).getByRole("button", { name: /^reply$/i }));
    await waitFor(() =>
      expect(callsTo(api, "POST", `${API}/comments/${asked.id}/replies`).map((call) => call.body)).toEqual([
        { text: "Thanks, that reads well." },
      ]),
    );
    await waitFor(() => expect(replies()).toHaveLength(3));
    expect(replies()[2]).toMatch(/You\s*just now\s*Thanks, that reads well\.$/);
  });

  test("Delete removes the comment and its pin straight away, without asking", async () => {
    const api = fakeApi();
    const confirm = vi.fn(() => false);
    vi.stubGlobal("confirm", confirm);
    const doomed = api.comments[1];
    renderAt(DOC_PATH);
    await findFrame();
    await select("p0000002");
    const comments = await section(COMMENTS);
    const card = (await within(comments).findByText("Say which week.")).closest("li") as HTMLElement;
    fireEvent.click(within(card).getByRole("button", { name: /^delete$/i }));

    await waitFor(() => expect(callsTo(api, "DELETE", `${API}/comments/${doomed.id}`)).toHaveLength(1));
    await waitFor(() => expect(within(comments).queryByText("Say which week.")).toBeNull());
    await shown((s) => expect(s.pins).toEqual([{ node: "p0000001", count: 1 }]));
    expect(confirm).not.toHaveBeenCalled();
  });

  test("?comment opens the doc on the comment's block, scrolled into view and selected, with its comments", async () => {
    const api = fakeApi();
    renderAt(`${DOC_PATH}?comment=${api.comments[1].id}`);
    await findFrame();
    const show = await shown((s) => expect(s.selected).toBe("p0000002"));
    expect(show.reveal, "the frame is told to scroll the block into view").toBe("p0000002");
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText("Say which week.")).toBeTruthy());
    expect(within(comments).queryByText("Is one day right?")).toBeNull();
  });

  test("?comment of a comment whose block was removed opens the doc at the top with nothing selected, the comment listed", async () => {
    const api = fakeApi();
    const gone = orphan("p0000009", "Shorten this note.", '<p data-omni-id="p0000009">Prices include VAT.</p>');
    api.comments = [...api.comments, gone];
    renderAt(`${DOC_PATH}?comment=${gone.id}`);
    await findFrame();
    await shown((s) => expect(s.html).toBe(HEAD_HTML));
    expect(await screen.findByText("Shorten this note.")).toBeTruthy();
    await new Promise((resolve) => setTimeout(resolve, 100));
    const latest = shows()[shows().length - 1];
    expect(latest.selected, "nothing is selected").toBeNull();
    expect(latest.reveal ?? null, "the frame stays at the top").toBeNull();
  });

  test("a ?comment the doc does not have opens the doc as without it", async () => {
    fakeApi();
    renderAt(`${DOC_PATH}?comment=0192d7a0-6666-7000-8000-0000000fffff`);
    await findFrame();
    await shown((s) => expect(s.html).toBe(HEAD_HTML));
    await new Promise((resolve) => setTimeout(resolve, 100));
    const latest = shows()[shows().length - 1];
    expect(latest.selected).toBeNull();
    expect(latest.reveal ?? null).toBeNull();
    expect(screen.queryByRole("alert"), "no error").toBeNull();
    expect(await within(await section(COMMENTS)).findByText(/select a block/i)).toBeTruthy();
  });

  test("a comment whose block was removed shows the block's tag and the start of its text as plain text, with a reply box and Delete", async () => {
    const api = fakeApi();
    const snapshot =
      '<p data-omni-id="p0000009"><img src="x" onerror="window.hacked = true">Refunds take <b>a week</b> from the day the parcel is back with us, and the money goes back the way it was paid, card or invoice.</p>';
    api.comments = [...api.comments, orphan("p0000009", "Shorten this note.", snapshot)];
    renderAt(DOC_PATH);
    await findFrame();
    const card = (await screen.findByText("Shorten this note.")).closest("li") as HTMLElement;
    expect(within(card).getByText("Element removed")).toBeTruthy();
    expect(within(card).getByText("<p>")).toBeTruthy();
    const text = within(card).getByText(/^Refunds take a week from the day the parcel is back with us/);
    expect(text.textContent, "only the start of the text").not.toMatch(/card or invoice/);
    expect(card.querySelector("img, b"), "the snapshot is not put into the page as HTML").toBeNull();
    expect(within(card).getByRole("textbox", { name: /^reply$/i })).toBeTruthy();
    expect(within(card).getByRole("button", { name: /^delete$/i })).toBeTruthy();
  });

  test("when an agent deletes the open doc, the page shows it as not found", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    const socket = await openSocket();
    await shown((s) => expect(s.html).toBe(HEAD_HTML));

    api.deleted = true;
    socket.receive(event(41, "doc.deleted", { doc_id: DOC_ID }));
    socket.receive(event(42, "doc.changed", { doc_id: DOC_ID, head: "0".repeat(64) }));
    await screen.findByText("The doc could not be loaded. Try again in a moment.");
    await waitFor(() => expect(document.querySelector("iframe"), "the deleted doc's content is gone").toBeNull());
    expect(screen.queryByText(DOC_TITLE), "and so is its title").toBeNull();
  });

  test("when an agent deletes another doc, the open doc stays shown", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    const frame = await findFrame();
    const socket = await openSocket();
    await shown((s) => expect(s.html).toBe(HEAD_HTML));

    socket.receive(event(41, "doc.deleted", { doc_id: "0192d7a0-5555-7000-8000-000000000009" }));
    const changed = HEAD_HTML.replace("Refunds take a week.", "Refunds take five days.");
    api.head = changed;
    socket.receive(event(42, "doc.changed", { doc_id: DOC_ID, agent: WRITER }));
    await shown((s) => expect(s.html).toBe(changed));
    expect(document.querySelector("iframe")).toBe(frame);
    expect(screen.getByRole("heading", { name: DOC_TITLE })).toBeTruthy();
    expect(screen.queryByText("The doc could not be loaded. Try again in a moment.")).toBeNull();
  });
});
