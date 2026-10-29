// @vitest-environment jsdom
// US-045: reviewing a design in the design viewer (US-043, US-044): comment
// pins on node addresses that follow the canvas, comments made on the
// selected node, and links from posts in the thread view (US-041) to the node
// in the right viewer. The viewer reads the doc's head (US-112): there is no
// version picker and no draft view. The app is rendered
// with its own router on a memory history, the browser's fetch is replaced by
// a fake of the API, and the browser's WebSocket by a fake socket that the
// test drives.
//
// The canvas is drawn by the renderer, which jsdom cannot run, so the page
// reaches it only through src/design/renderer.ts, which this test replaces by
// a fake. Besides what US-043 and US-044 use (show, layers, select, setMode,
// inspect, onSelect, destroy), the page uses two more methods:
//
//   // The box of the node at `address` on the canvas, in CSS pixels from the
//   // canvas's top-left corner, as the current zoom and pan show it (omni-
//   // render's `Scene::screen_box_of`, `_omni_render_node_box`,
//   // crates/omni-render/tests/pins.rs); `null` when the page shown has no
//   // node at that address.
//   boxOf(address: string): { x: number; y: number; width: number; height: number } | null;
//   // Calls `listener` whenever the view changes: a zoom, a pan, a resize or
//   // another page shown. Returns the function that stops it.
//   onView(listener: () => void): () => void;
//
// The fake's view starts at the origin at 100%; the test zooms and pans it
// and then calls the view listeners. The fake's `inspect` answers for any
// node of the page it was shown last (its `text` group has the text's
// `content`).
//
// The page:
//
// - Comment pins: over the canvas, one button per node address of the page
//   shown that has comments (on any version), whose accessible name says how
//   many comments it has and names the node by its layer name, for example
//   "2 comments on Total". A pin sits on its node's box (`boxOf`): its
//   position (inline `left`/`top` in px, or a `translate(<x>px, <y>px)`
//   transform, on the button or a wrapper up to three levels above it) is a
//   point on or right next to the box, relative to the canvas's top-left
//   corner, and it moves with the view. Clicking a pin selects its node; a
//   node selected on the canvas, in the layers panel or by a pin opens its
//   comments in a section named "Comments" (a heading names it).
// - A new comment: with a node selected, the Comments section has a field
//   named "Comment" and a button "Comment" (or "Add comment", or "Post") that
//   sends POST .../docs/<doc>/comments {node: <the selected node's address>,
//   text}; a node inside an instance is `<instanceId>/<componentNodeId>`.
// - Node links: a post's links of kind "node" ({kind: "node", doc_id,
//   address}) are shown in the thread view as links (router links, so the app
//   navigates without a reload) whose accessible name holds the node address
//   or the doc's title. Following one opens the doc's viewer, the design
//   viewer for a design and the doc viewer for an HTML doc, with the node
//   selected: in a design, the page that holds
//   the node is shown, the node is selected on the canvas and in the layers
//   panel, and its comments are open.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { answerDesignRead } from "./testing/design-boards";

type Layer = { address: string; name: string; type: string; children: Layer[] };
type Box = { x: number; y: number; width: number; height: number };
type Shown = { page: unknown; system: unknown };

/** What the fake renderers were asked to do. */
const fake = vi.hoisted(() => {
  type FakeLayer = { address: string; name: string; type: string; children: FakeLayer[] };
  type FakeBox = { x: number; y: number; width: number; height: number };
  type Instance = {
    canvas: HTMLCanvasElement;
    shown: { page: unknown; system: unknown }[];
    selected: (string | null)[];
    listeners: Set<(address: string | null) => void>;
    viewListeners: Set<() => void>;
    view: { x: number; y: number; zoom: number };
    destroyed: boolean;
  };
  return {
    instances: [] as Instance[],
    /** The layers the renderer gives for each page, by the page's ID. */
    layersByPage: new Map<string, FakeLayer[]>(),
    /** The page boxes of each page's nodes, by the page's ID and the node's address. */
    boxesByPage: new Map<string, Map<string, FakeBox>>(),
    /** The tags of the snapshots the renderer was asked to draw (`showTags`). */
    tagsShown: [] as string[],
  };
});

vi.mock("@/design/renderer", () => {
  type Node = { id?: string; name?: string; type?: string; content?: string; children?: Node[] };
  const find = (nodes: Node[], id: string): Node | null => {
    for (const node of nodes) {
      if (node.id === id) return node;
      const below = find(node.children ?? [], id);
      if (below) return below;
    }
    return null;
  };
  return {
    createDesignRenderer: async (canvas: HTMLCanvasElement) => {
      const instance = {
        canvas,
        shown: [] as Shown[],
        selected: [] as (string | null)[],
        listeners: new Set<(address: string | null) => void>(),
        viewListeners: new Set<() => void>(),
        view: { x: 0, y: 0, zoom: 1 },
        destroyed: false,
      };
      fake.instances.push(instance);
      const lastPage = () => instance.shown[instance.shown.length - 1]?.page as Node | undefined;
      return {
        show(page: unknown, system: unknown) {
          instance.shown.push({ page, system });
          for (const listener of Array.from(instance.viewListeners)) listener();
        },
        showTags(tags: string) {
          fake.tagsShown.push(tags);
        },
        layers() {
          const id = lastPage()?.id;
          return id ? (fake.layersByPage.get(id) ?? []) : [];
        },
        select(address: string | null) {
          instance.selected.push(address);
        },
        onSelect(listener: (address: string | null) => void) {
          instance.listeners.add(listener);
          return () => instance.listeners.delete(listener);
        },
        setMode() {},
        inspect(address: string) {
          const page = lastPage();
          const last = instance.shown[instance.shown.length - 1];
          if (!page || !last) return null;
          const segments = address.split("/");
          let node: Node | null;
          if (segments.length === 1) {
            node = find([page], address);
          } else {
            const components = ((last.system as { $components?: Node[] } | null)?.$components ?? []) as Node[];
            node = find([page], segments[0]) ? find(components, segments[segments.length - 1]) : null;
          }
          if (!node) return null;
          const box = fake.boxesByPage.get(page.id ?? "")?.get(address) ?? { x: 0, y: 0, width: 0, height: 0 };
          return {
            address,
            name: node.name ?? "",
            type: node.type ?? "",
            mode: null,
            box,
            layout: [],
            size: [],
            style: [],
            text: node.content === undefined ? [] : [{ name: "content", value: node.content, tokens: [] }],
            instance: null,
          };
        },
        boxOf(address: string) {
          const id = lastPage()?.id;
          const box = id ? fake.boxesByPage.get(id)?.get(address) : undefined;
          if (!box) return null;
          const { x, y, zoom } = instance.view;
          return { x: (box.x - x) * zoom, y: (box.y - y) * zoom, width: box.width * zoom, height: box.height * zoom };
        },
        onView(listener: () => void) {
          instance.viewListeners.add(listener);
          return () => instance.viewListeners.delete(listener);
        },
        // US-058's boards in view (src/design-boards.test.tsx): every board of the page shown.
        visibleBoards() {
          return (lastPage()?.children ?? []).map((board) => String(board.id));
        },
        // US-057's images and fonts (src/design-assets.test.tsx); this design has none.
        addImage() {},
        addFont() {},
        destroy() {
          instance.destroyed = true;
        },
      };
    },
  };
});

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const DOC_ID = "0192d7a0-7777-7000-8000-000000000001";
const HTML_DOC_ID = "0192d7a0-7777-7000-8000-000000000003";
const CS_OPEN = "0192d7a0-7777-7000-8000-000000000013";
const THREAD_ID = "0192d7a0-7777-7000-8000-000000000041";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const DOC_TITLE = "Checkout screens";
const HTML_TITLE = "Checkout PRD";
const DOC_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/docs/${DOC_ID}`;
const THREAD_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/threads/${THREAD_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T10:00:00Z";
const HUMAN = { kind: "human", user_id: USER.id };
const AUTHOR = { run: "run-1", role: "designer", model: "test-model", harness: "test-harness" };
const DESIGNER = { kind: "agent", ...AUTHOR };

// The design ---------------------------------------------------------------------------
//
// Two pages. Mobile holds the boards Cart (an instance of Button named Buy,
// and a frame Summary holding the text Total) and Receipt. Desktop holds the
// board Wide cart (the text Heading). Button's layers are Label (a text) and
// Icon (an instance of Cart, whose layer is Wheel).

const MOBILE = "pg000001";
const DESKTOP = "pg000002";
const CART_BOARD = "a0000001";
const RECEIPT = "a0000002";
const WIDE_CART = "a0000003";
const BUY = "e0000001";
const SUMMARY = "f0000001";
const TOTAL = "f0000002";
const HEADING = "f0000003";
const CART = "cc000001";
const WHEEL = "cw000001";
const BUTTON = "cb000001";
const LABEL = "ct000001";
const ICON = "cn000001";
const BUY_LABEL = `${BUY}/${LABEL}`;

const fixed = (value: number) => ({ type: "fixed", value });

function pages(total: string) {
  return [
    {
      id: MOBILE,
      type: "page",
      name: "Mobile",
      children: [
        {
          id: CART_BOARD,
          type: "board",
          name: "Cart",
          x: 0,
          y: 0,
          width: fixed(375),
          height: fixed(812),
          layout: { mode: "none" },
          children: [
            { id: BUY, type: "instance", name: "Buy", component: BUTTON, x: 20, y: 20 },
            {
              id: SUMMARY,
              type: "frame",
              name: "Summary",
              x: 20,
              y: 100,
              width: fixed(200),
              height: fixed(100),
              layout: { mode: "none" },
              children: [
                {
                  id: TOTAL,
                  type: "text",
                  name: "Total",
                  content: total,
                  fontFamily: "Inter",
                  fontSize: 14,
                  lineHeight: 1.25,
                },
              ],
            },
          ],
        },
        {
          id: RECEIPT,
          type: "board",
          name: "Receipt",
          x: 400,
          y: 0,
          width: fixed(375),
          height: fixed(812),
          layout: { mode: "none" },
        },
      ],
    },
    {
      id: DESKTOP,
      type: "page",
      name: "Desktop",
      children: [
        {
          id: WIDE_CART,
          type: "board",
          name: "Wide cart",
          x: 0,
          y: 0,
          width: fixed(1280),
          height: fixed(800),
          layout: { mode: "none" },
          children: [
            {
              id: HEADING,
              type: "text",
              name: "Heading",
              content: "Your cart",
              fontFamily: "Inter",
              fontSize: 32,
              lineHeight: 1.25,
            },
          ],
        },
      ],
    },
  ];
}

function components(label: string) {
  return [
    {
      id: CART,
      type: "component",
      name: "Cart",
      width: fixed(20),
      height: fixed(20),
      layout: { mode: "none" },
      children: [
        { id: WHEEL, type: "frame", name: "Wheel", width: fixed(6), height: fixed(6), layout: { mode: "none" } },
      ],
    },
    {
      id: BUTTON,
      type: "component",
      name: "Button",
      width: fixed(120),
      height: fixed(40),
      layout: { mode: "none" },
      fills: [{ type: "solid", color: "$color.brand" }],
      children: [
        { id: LABEL, type: "text", name: "Label", content: label, fontFamily: "Inter", fontSize: 16, lineHeight: 1.25 },
        { id: ICON, type: "instance", name: "Icon", component: CART },
      ],
    },
  ];
}

function tokens(brand: string) {
  return {
    $extensions: { omnistate: { modes: ["light", "dark"] } },
    color: {
      $type: "color",
      brand: {
        $value: brand,
        $extensions: { omnistate: { modes: { light: brand, dark: "#0000ff" } } },
      },
    },
  };
}

const HEAD_PAGES = pages("Total 120 kr");
const HEAD_COMPONENTS = components("Pay");
const HEAD_TOKENS = tokens("#cc0000");

function layer(address: string, name: string, type: string, children: Layer[] = []): Layer {
  return { address, name, type, children };
}

/** The layers and page boxes the renderer gives for each page: Button's layers sit under the instance Buy. */
function seedRenderer() {
  fake.layersByPage.set(MOBILE, [
    layer(CART_BOARD, "Cart", "board", [
      layer(BUY, "Buy", "instance", [
        layer(BUY_LABEL, "Label", "text"),
        layer(`${BUY}/${ICON}`, "Icon", "instance", [layer(`${BUY}/${WHEEL}`, "Wheel", "frame")]),
      ]),
      layer(SUMMARY, "Summary", "frame", [layer(TOTAL, "Total", "text")]),
    ]),
    layer(RECEIPT, "Receipt", "board"),
  ]);
  fake.layersByPage.set(DESKTOP, [layer(WIDE_CART, "Wide cart", "board", [layer(HEADING, "Heading", "text")])]);
  fake.boxesByPage.set(
    MOBILE,
    new Map<string, Box>([
      [CART_BOARD, { x: 0, y: 0, width: 375, height: 812 }],
      [BUY, { x: 20, y: 20, width: 120, height: 40 }],
      [BUY_LABEL, { x: 60, y: 30, width: 30, height: 20 }],
      [`${BUY}/${ICON}`, { x: 30, y: 30, width: 20, height: 20 }],
      [`${BUY}/${WHEEL}`, { x: 32, y: 42, width: 6, height: 6 }],
      [SUMMARY, { x: 20, y: 100, width: 200, height: 100 }],
      [TOTAL, { x: 20, y: 100, width: 80, height: 18 }],
      [RECEIPT, { x: 400, y: 0, width: 375, height: 812 }],
    ]),
  );
  fake.boxesByPage.set(
    DESKTOP,
    new Map<string, Box>([
      [WIDE_CART, { x: 0, y: 0, width: 1280, height: 800 }],
      [HEADING, { x: 0, y: 0, width: 150, height: 40 }],
    ]),
  );
}

// The API --------------------------------------------------------------------------------

let counter = 0;
function newId() {
  counter += 1;
  return `0192d7a0-6666-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

function comment(docId: string, node: string, text: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    doc_id: docId,
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

type Comment = ReturnType<typeof comment>;

const ON_TOTAL = "Is 96 right?";
const ON_LABEL = "The label reads too small";
const ON_HEADING = "Make the heading bolder";
const ON_REFUNDS = "Say which week.";

/**
 * The comments: on Total, on the Label inside Buy and on Heading (another
 * page) of the design, and one on a block of the HTML doc.
 */
function seededComments() {
  return [
    comment(DOC_ID, TOTAL, ON_TOTAL),
    comment(DOC_ID, BUY_LABEL, ON_LABEL),
    comment(DOC_ID, HEADING, ON_HEADING),
    comment(HTML_DOC_ID, "p0000002", ON_REFUNDS),
  ];
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

function threadPost(summary: string, links: Record<string, unknown>[]) {
  return {
    id: newId(),
    thread_id: THREAD_ID,
    phase: 1,
    type: "finding",
    author: DESIGNER,
    summary,
    body: "See the node.",
    body_html: "<p>See the node.</p>",
    links,
    attachments: [] as string[],
    confidence: null as number | null,
    replaces: null as string | null,
    created_at: AT,
  };
}

const POST_ON_LABEL = "The label is cramped";
const POST_ON_HEADING = "The heading needs weight";
const POST_ON_REFUNDS = "The refund line is vague";

function seededThread() {
  const posts = [
    threadPost(POST_ON_LABEL, [{ kind: "node", doc_id: DOC_ID, address: BUY_LABEL }]),
    threadPost(POST_ON_HEADING, [{ kind: "node", doc_id: DOC_ID, address: HEADING }]),
    threadPost(POST_ON_REFUNDS, [{ kind: "node", doc_id: HTML_DOC_ID, address: "p0000002" }]),
  ];
  return {
    id: THREAD_ID,
    project_id: PROJECT_ID,
    changeset_id: CS_OPEN,
    question: "Is the checkout ready?",
    criteria: ["Readable"],
    run_tag: "run-1",
    created_at: AT,
    phases: [{ number: 1, sealed: false, started_at: AT, posted_by: [{ run: "run-1", role: "designer" }] }],
    decision: null,
    posts,
  };
}

const HTML_HEAD = `<section data-omni-id="s0000001">
<h1 data-omni-id="h0000001">Checkout</h1>
<p data-omni-id="p0000001">Orders ship in two days.</p>
<p data-omni-id="p0000002">Refunds take a week.</p>
</section>`;

type Call = { method: string; path: string; body: Record<string, unknown> | null };

/**
 * A fake of the API: the auth routes, the project, the design doc with its
 * head and comments, an HTML doc and a thread.
 */
function fakeApi() {
  const api = {
    comments: seededComments() as Comment[],
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
    kind: "design",
    pillar: "ui",
    title: DOC_TITLE,
    created_at: AT,
    drafts: [],
    claims: [],
  };
  const htmlDoc = {
    id: HTML_DOC_ID,
    project_id: PROJECT_ID,
    kind: "html",
    pillar: "spec",
    title: HTML_TITLE,
    created_at: AT,
    drafts: [],
    claims: [],
  };
  const changesets = [changeset(CS_OPEN, "Add a receipt", "open")];
  const head = { pages: HEAD_PAGES, components: HEAD_COMPONENTS, tokens: HEAD_TOKENS };
  const thread = seededThread();

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
    api.calls.push({ method, path, body });

    // US-058 and US-112: the head's outline and its boards, one by one.
    const read = answerDesignRead(API, method, url, () => null, (id) => (id === DOC_ID ? head : null));
    if (read) return json(read.status, read.body);
    switch (`${method} ${path}`) {
      case "GET /api/v1/auth/me":
        return json(200, { user: USER });
      case "GET /api/v1/auth/workspaces":
        return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
      case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
        return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
      case `GET ${API}/home`: {
        const listed: Record<string, unknown> = { ...thread, decision: null };
        delete listed.posts;
        return json(200, {
          project_id: PROJECT_ID,
          docs: [doc, htmlDoc],
          changesets,
          threads: [listed],
          waiting: { decisions: [], changesets: [], comments: [], questions: [] },
        });
      }
      case `GET ${API}/changesets`:
        return json(200, { changesets });
      case `GET ${API}/docs/${DOC_ID}`:
        return json(200, { doc });
      case `GET ${API}/docs/${HTML_DOC_ID}`:
        return json(200, { doc: htmlDoc });
      case `GET ${API}/docs/${HTML_DOC_ID}/html`:
        return json(200, { doc_id: HTML_DOC_ID, html: HTML_HEAD });
      case `GET ${API}/comments`: {
        const docFilter = url.searchParams.get("doc");
        const listed = api.comments.filter((c) => !docFilter || c.doc_id === docFilter);
        return json(200, { project_id: PROJECT_ID, comments: listed });
      }
      case `GET ${API}/threads/${THREAD_ID}`:
        return json(200, { thread });
      case `POST ${API}/docs/${DOC_ID}/comments`:
      case `POST ${API}/docs/${HTML_DOC_ID}/comments`: {
        const node = typeof body?.node === "string" ? body.node : "";
        const said = typeof body?.text === "string" ? body.text.trim() : "";
        if (!node || !said) return json(400, { error: "invalid_request" });
        const created = comment(path.split("/").slice(-2)[0], node, said);
        api.comments = [...api.comments, created];
        return json(201, { comment: created });
      }
      default:
        break;
    }
    const single = changesets.find((c) => path === `${API}/changesets/${c.id}`);
    if (method === "GET" && single) return json(200, { changeset: single });
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

// Reading the page -----------------------------------------------------------------------

type FakeRenderer = (typeof fake.instances)[number];

/**
 * The renderer of the design canvas: the first one the page made that is
 * still in use and whose canvas is on the page (a renderer the page makes
 * later, to show a node of another version, comes after it).
 */
async function findRenderer(): Promise<FakeRenderer> {
  return waitFor(() => {
    const found = fake.instances.find((instance) => !instance.destroyed && document.body.contains(instance.canvas));
    expect(found, "the page made a renderer for its canvas (createDesignRenderer)").toBeTruthy();
    return found!;
  });
}

/** Waits until the canvas renderer's last shown page passes `check`, and returns what it was given. */
async function findShown(check: (shown: Shown) => void): Promise<Shown> {
  const renderer = await findRenderer();
  return waitFor(() => {
    const last = renderer.shown[renderer.shown.length - 1];
    expect(last, "the page showed a page of the design on the renderer").toBeTruthy();
    check(last);
    return last;
  });
}

function pageId(shown: Shown) {
  return (shown.page as { id?: string }).id;
}

/** The reviewer selects `address` on the canvas. */
async function selectOnCanvas(address: string | null) {
  const renderer = await findRenderer();
  await waitFor(() => expect(renderer.listeners.size, "the page listens to the canvas").toBeGreaterThan(0));
  act(() => {
    for (const listener of Array.from(renderer.listeners)) listener(address);
  });
}

/** Zooms and pans the canvas renderer's view and tells the page. */
async function moveView(view: { x: number; y: number; zoom: number }) {
  const renderer = await findRenderer();
  await waitFor(() => expect(renderer.viewListeners.size, "the page follows the view (onView)").toBeGreaterThan(0));
  act(() => {
    renderer.view = view;
    for (const listener of Array.from(renderer.viewListeners)) listener();
  });
}

/** The page's section (or group) named `name`. */
async function section(name: RegExp) {
  return waitFor(() => {
    const found = screen.queryByRole("region", { name }) ?? screen.queryByRole("group", { name });
    expect(found, `a section named ${name}`).toBeTruthy();
    return found as HTMLElement;
  });
}

const COMMENTS = /^comments\b/i;

/** The comment pins over the canvas. */
function pins() {
  return screen.queryAllByRole("button", { name: /\bcomments? on\b/i });
}

function nameOf(element: HTMLElement) {
  return element.getAttribute("aria-label") ?? element.textContent ?? "";
}

/** The pin of the node named `name`, as the page is now. */
function pinOn(name: string) {
  const found = pins().filter((pin) => new RegExp(`\\b${name}\\b`).test(nameOf(pin)));
  expect(found.length, `one pin on ${name}; pins: ${pins().map(nameOf).join(", ")}`).toBe(1);
  return found[0];
}

/** The pin of the node named `name`, once the page shows it. */
async function findPin(name: string) {
  return waitFor(() => pinOn(name));
}

/** Where a pin sits, from its inline position or translate transform (or a wrapper's). */
function pinPoint(pin: HTMLElement): [number, number] {
  let element: HTMLElement | null = pin;
  for (let level = 0; element && level <= 3; level += 1, element = element.parentElement) {
    const { left, top, transform } = element.style;
    if (left && top) return [parseFloat(left), parseFloat(top)];
    const moved = /translate(?:3d)?\(\s*(-?[\d.]+)px\s*,\s*(-?[\d.]+)px/.exec(transform);
    if (moved) return [parseFloat(moved[1]), parseFloat(moved[2])];
  }
  throw new Error("a pin is placed with inline left/top or a translate transform");
}

/** Whether a point is on the box, or within `margin` pixels of it. */
function nearBox([x, y]: [number, number], box: Box, margin = 24) {
  return (
    x >= box.x - margin && x <= box.x + box.width + margin && y >= box.y - margin && y <= box.y + box.height + margin
  );
}

/** Writes a comment in the comments section and sends it. */
async function writeComment(text: string) {
  const comments = await section(COMMENTS);
  const field = await within(comments).findByRole("textbox", { name: /comment/i });
  fireEvent.change(field, { target: { value: text } });
  fireEvent.click(within(comments).getByRole("button", { name: /^(add comment|comment|post)$/i }));
}

function treeItem(name: string) {
  const region = screen.getByRole("region", { name: /^layers$/i });
  return within(region).getByRole("treeitem", { name: new RegExp(`^${name}\\b`) });
}

function callsTo(api: ReturnType<typeof fakeApi>, method: string, path: string) {
  return api.calls.filter((call) => call.method === method && call.path === path);
}

let api: ReturnType<typeof fakeApi>;

beforeEach(() => {
  fake.instances.length = 0;
  fake.layersByPage.clear();
  fake.boxesByPage.clear();
  fake.tagsShown.length = 0;
  seedRenderer();
  api = fakeApi();
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

// Pins -----------------------------------------------------------------------------------

describe("comment pins", () => {
  test("a pin sits on each commented node of the page shown, nodes inside instances included", async () => {
    renderAt(DOC_PATH);
    await findShown((shown) => expect(pageId(shown)).toBe(MOBILE));
    const total = await findPin("Total");
    const label = await findPin("Label");
    expect(
      nearBox(pinPoint(total), { x: 20, y: 100, width: 80, height: 18 }),
      `the pin on Total sits on its box: ${pinPoint(total).join(", ")}`,
    ).toBe(true);
    expect(
      nearBox(pinPoint(label), { x: 60, y: 30, width: 30, height: 20 }),
      `the pin on the Label inside Buy sits on that node's box: ${pinPoint(label).join(", ")}`,
    ).toBe(true);
    expect(
      pins().filter((pin) => /\bHeading\b/.test(nameOf(pin))),
      "Heading is on another page, so it has no pin here",
    ).toEqual([]);
  });

  test("pins follow zooming and panning", async () => {
    renderAt(DOC_PATH);
    await findShown((shown) => expect(pageId(shown)).toBe(MOBILE));
    const [x0, y0] = pinPoint(await findPin("Total"));

    // A pan of 100 px right and 40 px down moves the pin as far.
    await moveView({ x: -100, y: -40, zoom: 1 });
    await waitFor(() => {
      const [x, y] = pinPoint(pinOn("Total"));
      expect([x - x0, y - y0], "the pin moved with the pan").toEqual([100, 40]);
    });

    // At 200% from the origin, Total's box is (40, 200, 160 x 36).
    await moveView({ x: 0, y: 0, zoom: 2 });
    await waitFor(() => {
      const point = pinPoint(pinOn("Total"));
      expect(
        nearBox(point, { x: 40, y: 200, width: 160, height: 36 }),
        `the pin is on the zoomed box: ${point.join(", ")}`,
      ).toBe(true);
    });
  });

  test("clicking a pin selects its node and opens its comments", async () => {
    renderAt(DOC_PATH);
    const renderer = await findRenderer();
    fireEvent.click(await findPin("Label"));
    await waitFor(() =>
      expect(renderer.selected[renderer.selected.length - 1], "the canvas selects the node inside the instance").toBe(
        BUY_LABEL,
      ),
    );
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText(ON_LABEL)).toBeTruthy());
    expect(within(comments).queryByText(ON_TOTAL), "only the selected node's comments").toBeNull();
  });

  test("clicking a node on the canvas opens its comments", async () => {
    renderAt(DOC_PATH);
    await findShown((shown) => expect(pageId(shown)).toBe(MOBILE));
    await selectOnCanvas(TOTAL);
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText(ON_TOTAL)).toBeTruthy());
    expect(within(comments).queryByText(ON_LABEL)).toBeNull();

    await selectOnCanvas(BUY_LABEL);
    await waitFor(() => expect(within(comments).getByText(ON_LABEL)).toBeTruthy());
    expect(within(comments).queryByText(ON_TOTAL)).toBeNull();
  });
});

// New comments ---------------------------------------------------------------------------

describe("a new comment", () => {
  test("attaches to the selected node address of the doc, inside an instance", async () => {
    renderAt(DOC_PATH);
    await findShown((shown) => expect(pageId(shown)).toBe(MOBILE));
    await selectOnCanvas(BUY_LABEL);
    await writeComment("Use the brand font");
    await waitFor(() => expect(callsTo(api, "POST", `${API}/docs/${DOC_ID}/comments`)).toHaveLength(1));
    expect(callsTo(api, "POST", `${API}/docs/${DOC_ID}/comments`)[0].body).toEqual({
      node: BUY_LABEL,
      text: "Use the brand font",
    });
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText("Use the brand font")).toBeTruthy());
  });
});

// Links from posts -----------------------------------------------------------------------

/** The node link of the post whose summary is `summary`, in the thread view. */
async function findNodeLink(summary: string, names: RegExp) {
  return waitFor(() => {
    const said = screen.getByText(summary);
    const post = said.closest("li, article");
    expect(post, `the post "${summary}" is a list item or article`).toBeTruthy();
    const links = within(post as HTMLElement)
      .queryAllByRole("link")
      .filter((link) => names.test(link.textContent ?? "") || names.test(link.getAttribute("aria-label") ?? ""));
    expect(links.length, `the post "${summary}" shows its node link, named ${names}`).toBe(1);
    return links[0];
  });
}

describe("links to node addresses from posts", () => {
  test("open a node inside an instance in the design viewer, selected, with its comments", async () => {
    renderAt(THREAD_PATH);
    const link = await findNodeLink(POST_ON_LABEL, new RegExp(`${DOC_TITLE}|${BUY}/${LABEL}`));
    fireEvent.click(link);
    const renderer = await findRenderer();
    await findShown((shown) => expect(pageId(shown)).toBe(MOBILE));
    await waitFor(() =>
      expect(renderer.selected[renderer.selected.length - 1], "the node is selected on the canvas").toBe(BUY_LABEL),
    );
    await waitFor(() => expect(treeItem("Label").getAttribute("aria-selected")).toBe("true"));
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText(ON_LABEL)).toBeTruthy());
  });

  test("open the page that holds the node", async () => {
    renderAt(THREAD_PATH);
    const link = await findNodeLink(POST_ON_HEADING, new RegExp(`${DOC_TITLE}|${HEADING}`));
    fireEvent.click(link);
    const renderer = await findRenderer();
    await findShown((shown) => expect(pageId(shown), "Heading is on the Desktop page").toBe(DESKTOP));
    await waitFor(() => expect(renderer.selected[renderer.selected.length - 1]).toBe(HEADING));
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText(ON_HEADING)).toBeTruthy());
  });

  test("open a block of an HTML doc in the doc viewer, selected, with its comments", async () => {
    renderAt(THREAD_PATH);
    const link = await findNodeLink(POST_ON_REFUNDS, new RegExp(`${HTML_TITLE}|p0000002`));
    fireEvent.click(link);
    expect(await screen.findByRole("heading", { name: HTML_TITLE })).toBeTruthy();
    await waitFor(() => expect(document.querySelector("iframe"), "the HTML doc's frame").toBeTruthy());
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText(ON_REFUNDS), "the block's comments are open").toBeTruthy());
    expect(fake.instances.filter((instance) => !instance.destroyed), "no design canvas for an HTML doc").toEqual([]);
  });
});

// Links to comments ----------------------------------------------------------------------

describe("links to comments", () => {
  test("?comment shows the page of the comment's node, selects the node and opens its comments", async () => {
    const onHeading = api.comments.find((c) => c.text === ON_HEADING)!;
    renderAt(`${DOC_PATH}?comment=${onHeading.id}`);
    const renderer = await findRenderer();
    await findShown((shown) => expect(pageId(shown), "Heading is on the Desktop page").toBe(DESKTOP));
    await waitFor(() => expect(renderer.selected[renderer.selected.length - 1]).toBe(HEADING));
    await waitFor(() => expect(treeItem("Heading").getAttribute("aria-selected")).toBe("true"));
    const comments = await section(COMMENTS);
    await waitFor(() => expect(within(comments).getByText(ON_HEADING)).toBeTruthy());
  });

  test("?comment of a comment whose node was removed opens the first page with nothing selected, and shows the node's picture and name", async () => {
    const tags = '<Text id="f0000009" name="Old total" content="Total 96 kr" fontFamily="Inter" fontSize={14}/>';
    const gone = { ...comment(DOC_ID, "f0000009", "Show the VAT too."), orphaned: true, snapshot: tags };
    api.comments = [...api.comments, gone];
    renderAt(`${DOC_PATH}?comment=${gone.id}`);
    const renderer = await findRenderer();
    await findShown((shown) => expect(pageId(shown)).toBe(MOBILE));
    const card = (await screen.findByText("Show the VAT too.")).closest("li") as HTMLElement;
    expect(within(card).getByText("Element removed")).toBeTruthy();
    expect(within(card).getByText("Old total")).toBeTruthy();
    await waitFor(() => expect(fake.tagsShown, "the renderer draws the node from its tag form").toContain(tags));
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(renderer.selected.filter((address) => address !== null), "nothing is selected").toEqual([]);
    expect(pageId(renderer.shown[renderer.shown.length - 1])).toBe(MOBILE);
  });

  test("a ?comment the doc does not have opens the first page with nothing selected", async () => {
    renderAt(`${DOC_PATH}?comment=0192d7a0-6666-7000-8000-0000000fffff`);
    const renderer = await findRenderer();
    await findShown((shown) => expect(pageId(shown)).toBe(MOBILE));
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(renderer.selected.filter((address) => address !== null), "nothing is selected").toEqual([]);
    expect(pageId(renderer.shown[renderer.shown.length - 1])).toBe(MOBILE);
    expect(screen.queryByRole("alert"), "no error").toBeNull();
  });
});
