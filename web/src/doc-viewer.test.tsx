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
// The page loads the doc (GET .../docs/<doc>, with its drafts and live
// claims), its versions (GET .../docs/<doc>/versions, crates/omni-api/tests/
// doc_versions.rs: `{doc_id, versions}`, oldest first, each with `id`,
// `doc_id`, `changeset_id`, `draft_id`, `removed`, `created_at` and `agent`),
// the HTML of the version shown (GET .../docs/<doc>/html?version=<id>, or
// without `version` for the mainline version) and the doc's comments (GET
// .../comments?doc=<doc>). It shows:
//
// - the doc's title as a heading;
// - a version picker: a select named "Version" with one option per saved
//   version, whose value is the version's ID, and one option per draft of the
//   doc, whose text says "draft" (the live draft of that change set). The
//   mainline version is shown first;
// - a section named "Comments" (a heading names it) with the comments of the
//   selected block and, once a block is selected, a field named "Comment" and
//   a button "Comment" (or "Add comment", or "Post") that sends POST
//   .../versions/<version shown>/comments {node, text}. A comment made on an
//   older version than the one shown shows that version of its block (read
//   from GET .../docs/<doc>/html?version=<the comment's version>) next to the
//   block as it is now;
// - in the live draft (GET .../changesets/<change set>/docs/<doc>), the draft
//   as it changes: the page subscribes to the project's live events and loads
//   the draft again when an operation is applied (`op.applied`), and it shows
//   the draft's claims, each holder named `<run>/<role>`, as they are taken and
//   released (`claim.taken`, `claim.released`, `claim.expired`).
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { viewerPolicyProblems } from "./testing/viewer-policy";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const DOC_ID = "0192d7a0-5555-7000-8000-000000000001";
const CS_FIRST = "0192d7a0-5555-7000-8000-000000000011";
const CS_SECOND = "0192d7a0-5555-7000-8000-000000000012";
const CS_OPEN = "0192d7a0-5555-7000-8000-000000000013";
const V1 = "0192d7a0-5555-7000-8000-000000000021";
const V2 = "0192d7a0-5555-7000-8000-000000000022";
const DRAFT_ID = "0192d7a0-5555-7000-8000-000000000031";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const DOC_TITLE = "Checkout PRD";
const DOC_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/docs/${DOC_ID}`;
const HOME_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T10:00:00Z";
const HUMAN = { kind: "human", user_id: USER.id };
const AUTHOR = { run: "run-1", role: "author", model: "test-model", harness: "test-harness" };
const WRITER = { run: "run-1", role: "writer", model: "test-model", harness: "test-harness" };

const V1_HTML = `<section data-node-id="s0000001">
<h1 data-node-id="h0000001">Checkout</h1>
<p data-node-id="p0000001">Orders ship in two days.</p>
<p data-node-id="p0000002">Refunds take a week.</p>
</section>`;
const V2_HTML = V1_HTML.replace("Orders ship in two days.", "Orders ship in one day.");

function draftHtml(extra: string) {
  return `<section data-node-id="s0000001" data-rev="3">
<h1 data-node-id="h0000001" data-rev="1">Checkout</h1>
<p data-node-id="p0000001" data-rev="2">Orders ship in one day.</p>
<p data-node-id="p0000002" data-rev="1">Refunds take a week.</p>
${extra}
</section>`;
}

function version(id: string, changesetId: string, createdAt: string) {
  return {
    id,
    doc_id: DOC_ID,
    changeset_id: changesetId,
    draft_id: `${id.slice(0, -2)}99`,
    removed: false,
    created_at: createdAt,
    agent: AUTHOR,
  };
}

function changeset(id: string, title: string, status: string) {
  return {
    id,
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

function claim(node: string) {
  return {
    id: "0192d7a0-5555-7000-8000-000000000041",
    doc_id: DOC_ID,
    draft_id: DRAFT_ID,
    changeset_id: CS_OPEN,
    node,
    agent: WRITER,
    taken_at: AT,
    expires_at: "2026-10-21T10:10:00Z",
  };
}

let counter = 0;
function newId() {
  counter += 1;
  return `0192d7a0-6666-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

type Comment = ReturnType<typeof comment>;

function comment(versionId: string, node: string, text: string, nodeStatus: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    doc_id: DOC_ID,
    version_id: versionId,
    node,
    changeset_id: null as string | null,
    author: HUMAN,
    text,
    status: "open",
    addressed_in: null as string | null,
    replies: [] as unknown[],
    node_status: nodeStatus,
    compared_to: V2 as string | null,
    created_at: AT,
  };
}

/** The doc's two comments: one made on the older version, on a block that changed since, and one on the mainline. */
function seededComments() {
  return [
    comment(V1, "p0000001", "Is two days right?", "changed"),
    comment(V2, "p0000002", "Say which week.", "current"),
  ];
}

type Call = { method: string; path: string; search: URLSearchParams; body: Record<string, unknown> | null };

/** A fake of the API: the auth routes, the project, the doc with its versions, draft and comments, and the calls it got. */
function fakeApi() {
  const api = {
    comments: seededComments() as Comment[],
    claims: [] as ReturnType<typeof claim>[],
    draft: draftHtml('<p data-node-id="p0000003" data-rev="1">Returns are free.</p>'),
    draftLoads: 0,
    calls: [] as Call[],
  };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  const doc = () => ({
    id: DOC_ID,
    project_id: PROJECT_ID,
    kind: "html",
    module: "prd",
    title: DOC_TITLE,
    created_at: AT,
    mainline_version_id: V2,
    drafts: [{ id: DRAFT_ID, doc_id: DOC_ID, changeset_id: CS_OPEN, base_version_id: V2 }],
    claims: api.claims,
  });
  const changesets = [
    changeset(CS_FIRST, "Write the checkout PRD", "merged"),
    changeset(CS_SECOND, "Ship faster", "merged"),
    changeset(CS_OPEN, "Add a returns policy", "open"),
  ];
  const versions = [version(V1, CS_FIRST, "2026-10-20T09:00:00Z"), version(V2, CS_SECOND, "2026-10-21T09:00:00Z")];
  const htmlOf: Record<string, string> = { [V1]: V1_HTML, [V2]: V2_HTML };

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
          docs: [doc()],
          changesets,
          threads: [],
          waiting: { decisions: [], changesets: [], comments: [], questions: [] },
        });
      case `GET ${API}/changesets`:
        return json(200, { changesets });
      case `GET ${API}/docs/${DOC_ID}`:
        return json(200, { doc: doc() });
      case `GET ${API}/docs/${DOC_ID}/versions`:
        return json(200, { doc_id: DOC_ID, versions });
      case `GET ${API}/docs/${DOC_ID}/html`: {
        const id = url.searchParams.get("version") ?? V2;
        if (!htmlOf[id]) return json(404, { error: "not_found" });
        return json(200, { doc_id: DOC_ID, version_id: id, html: htmlOf[id] });
      }
      case `GET ${API}/changesets/${CS_OPEN}/docs/${DOC_ID}`:
        api.draftLoads += 1;
        return json(200, {
          draft: { id: DRAFT_ID, doc_id: DOC_ID, changeset_id: CS_OPEN, base_version_id: V2, html: api.draft },
        });
      case `GET ${API}/comments`: {
        const docFilter = url.searchParams.get("doc");
        const listed = api.comments.filter((c) => !docFilter || c.doc_id === docFilter);
        return json(200, { project_id: PROJECT_ID, comments: listed });
      }
      default:
        break;
    }
    const single = changesets.find((c) => path === `${API}/changesets/${c.id}`);
    if (method === "GET" && single) return json(200, { changeset: single });
    const read = versions.find((v) => path === `${API}/versions/${v.id}`);
    if (method === "GET" && read) {
      const nodes = Array.from(htmlOf[read.id].matchAll(/<(\w+)[^>]*data-node-id="(\w+)"/g), (m) => ({
        id: m[2],
        type: m[1] === "h1" ? "heading" : m[1] === "p" ? "paragraph" : "section",
        parent: m[2] === "s0000001" ? null : "s0000001",
        fingerprint: `${read.id}-${m[2]}`,
      }));
      return json(200, { version: { ...read, design_system_version_id: null, nodes } });
    }
    const commentOn = versions.find((v) => path === `${API}/versions/${v.id}/comments`);
    if (method === "POST" && commentOn) {
      const node = typeof body?.node === "string" ? body.node : "";
      const said = typeof body?.text === "string" ? body.text.trim() : "";
      if (!node || !said) return json(400, { error: "invalid_request" });
      const created = comment(commentOn.id, node, said, "current");
      api.comments = [...api.comments, created];
      return json(201, { comment: created });
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

async function versionPicker() {
  return (await screen.findByRole("combobox", { name: /^version\b/i })) as HTMLSelectElement;
}

/** Picks the option of the version picker whose value is `value`. */
async function pick(value: string) {
  const picker = await versionPicker();
  fireEvent.change(picker, { target: { value } });
  expect(picker.value).toBe(value);
}

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
    vi.unstubAllGlobals();
  });

  test("the project home links each doc to its viewer", async () => {
    fakeApi();
    const router = renderAt(HOME_PATH);
    const link = await screen.findByRole("link", { name: DOC_TITLE });
    expect(link.getAttribute("href")).toBe(DOC_PATH);
    fireEvent.click(link);
    await waitFor(() => expect(router.state.location.pathname).toBe(DOC_PATH));
    expect(await screen.findByRole("heading", { name: DOC_TITLE })).toBeTruthy();
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

  test("the frame is shown the mainline version, and the picker lists every version and draft", async () => {
    fakeApi();
    renderAt(DOC_PATH);
    expect(await screen.findByRole("heading", { name: DOC_TITLE })).toBeTruthy();
    await findFrame();
    const show = await shown((s) => expect(s.html).toContain("Orders ship in one day."));
    expect(show.html).toContain('data-node-id="p0000002"');

    const picker = await versionPicker();
    await waitFor(() => expect(picker.value).toBe(V2));
    const options = Array.from(picker.options);
    expect(options.map((o) => o.value)).toEqual(expect.arrayContaining([V1, V2]));
    expect(
      options.filter((o) => /draft/i.test(o.textContent ?? "")).length,
      "one option for the doc's draft",
    ).toBe(1);
  });

  test("picking an older version shows that version", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    await shown((s) => expect(s.html).toContain("Orders ship in one day."));

    await pick(V1);
    await shown((s) => {
      expect(s.html).toContain("Orders ship in two days.");
      expect(s.html).not.toContain("Orders ship in one day.");
    });
    expect(
      callsTo(api, "GET", `${API}/docs/${DOC_ID}/html`).some((call) => call.search.get("version") === V1),
    ).toBe(true);
  });

  test("selecting a block in the frame opens its comments, and the frame is told it is selected", async () => {
    fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    await select("p0000002");

    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText("Say which week.")).toBeTruthy());
    expect(within(comments).queryByText("Is two days right?")).toBeNull();
    await shown((s) => expect(s.selected).toBe("p0000002"));

    // A message from any other window is not the frame's.
    fromFrame({ type: "viewer.select", node: "p0000001" }, window);
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(within(comments).getByText("Say which week.")).toBeTruthy();
    expect(within(comments).queryByText("Is two days right?")).toBeNull();
    expect(shows()[shows().length - 1].selected).toBe("p0000002");
  });

  test("pins mark the blocks that have comments", async () => {
    fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    const show = await shown((s) => {
      expect(s.html).toContain("Orders ship in one day.");
      expect(s.pins).toContainEqual({ node: "p0000002", count: 1 });
    });
    expect(show.pins.filter((pin) => pin.node === "h0000001" && pin.count > 0)).toEqual([]);
    expect(show.pins.filter((pin) => pin.node === "s0000001" && pin.count > 0)).toEqual([]);
  });

  test("a new comment attaches to the selected block and the version shown", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    await select("p0000002");
    await writeComment("Name the week in days.");
    await waitFor(() =>
      expect(callsTo(api, "POST", `${API}/versions/${V2}/comments`).map((call) => call.body)).toEqual([
        { node: "p0000002", text: "Name the week in days." },
      ]),
    );
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText("Name the week in days.")).toBeTruthy());
    await shown((s) => expect(s.pins).toContainEqual({ node: "p0000002", count: 2 }));

    await pick(V1);
    await shown((s) => expect(s.html).toContain("Orders ship in two days."));
    await select("p0000001");
    await writeComment("Two days was right.");
    await waitFor(() =>
      expect(callsTo(api, "POST", `${API}/versions/${V1}/comments`).map((call) => call.body)).toEqual([
        { node: "p0000001", text: "Two days was right." },
      ]),
    );
  });

  test("a comment made on an older version shows that version of the block next to the current one", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    await shown((s) => expect(s.html).toContain("Orders ship in one day."));
    await select("p0000001");

    const comments = await section(COMMENTS);
    await waitFor(() => {
      expect(within(comments).getByText("Is two days right?")).toBeTruthy();
      expect(comments.textContent).toContain("Orders ship in two days.");
      expect(comments.textContent).toContain("Orders ship in one day.");
    });
    expect(
      callsTo(api, "GET", `${API}/docs/${DOC_ID}/html`).some((call) => call.search.get("version") === V1),
    ).toBe(true);
  });

  test("the live draft follows the agents' operations", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    const socket = await openSocket();

    const picker = await versionPicker();
    const draft = await waitFor(() => {
      const option = Array.from(picker.options).find((o) => /draft/i.test(o.textContent ?? ""));
      expect(option, "an option for the draft").toBeTruthy();
      return option as HTMLOptionElement;
    });
    await pick(draft.value);
    await shown((s) => expect(s.html).toContain("Returns are free."));
    expect(api.draftLoads).toBeGreaterThan(0);

    const before = api.draftLoads;
    api.draft = draftHtml(
      '<p data-node-id="p0000003" data-rev="1">Returns are free.</p>\n<p data-node-id="p0000004" data-rev="1">Coupons stack.</p>',
    );
    socket.receive(
      event(41, "op.applied", {
        doc_id: DOC_ID,
        changeset_id: CS_OPEN,
        draft_id: DRAFT_ID,
        node: "p0000004",
        revision: 1,
        created: ["p0000004"],
        removed: [],
        agent: WRITER,
      }),
    );
    await shown((s) => expect(s.html).toContain("Coupons stack."));
    expect(api.draftLoads).toBeGreaterThan(before);
  });

  test("the live draft shows claims as they are taken and released", async () => {
    const api = fakeApi();
    renderAt(DOC_PATH);
    await findFrame();
    const socket = await openSocket();
    const picker = await versionPicker();
    const draft = await waitFor(() => {
      const option = Array.from(picker.options).find((o) => /draft/i.test(o.textContent ?? ""));
      expect(option, "an option for the draft").toBeTruthy();
      return option as HTMLOptionElement;
    });
    await pick(draft.value);
    await shown((s) => expect(s.html).toContain("Returns are free."));
    expect(screen.queryByText(/run-1\/writer/)).toBeNull();

    const held = claim("s0000001");
    api.claims = [held];
    socket.receive(
      event(41, "claim.taken", {
        claim_id: held.id,
        doc_id: DOC_ID,
        draft_id: DRAFT_ID,
        changeset_id: CS_OPEN,
        node: "s0000001",
        agent: WRITER,
      }),
    );
    expect(await screen.findByText(/run-1\/writer/)).toBeTruthy();

    api.claims = [];
    socket.receive(
      event(42, "claim.released", {
        claim_id: held.id,
        doc_id: DOC_ID,
        draft_id: DRAFT_ID,
        changeset_id: CS_OPEN,
        node: "s0000001",
        agent: WRITER,
      }),
    );
    await waitFor(() => expect(screen.queryByText(/run-1\/writer/)).toBeNull());
  });
});
