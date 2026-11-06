// @vitest-environment jsdom
// US-043: the design viewer, where the reviewer browses a design doc's pages
// and boards on the renderer's canvas, with a layers panel that stays in step
// with the canvas. It is the doc page, /w/<workspace id>/projects/<project
// id>/docs/<doc id> (the project home links every doc's title there), for a
// doc whose `kind` is "design": the page shows the design on a canvas instead
// of the HTML doc's frame. The app is rendered with its own router on a memory
// history, the browser's fetch is replaced by a fake of the API, and the
// browser's WebSocket by a fake socket.
//
// The page loads the doc (GET .../docs/<doc>) and its head (US-112), board by
// board (src/testing/design-boards.ts): its pages, `components` (the
// components its instances use, from the design system) and `tokens` (the
// design system's whole DTCG token tree, crates/omni-api/tests/components.rs).
//
// The canvas is drawn by the Phase 0 renderer (omni-render, WebAssembly on
// WebGL), which jsdom cannot run. The page reaches it only through the module
// src/design/renderer.ts, which this test replaces with a fake that records
// what the page asks of it:
//
//   export type Layer = { address: string; name: string; type: string; children: Layer[] };
//   export type DesignRenderer = {
//     // Shows `page` (one of the version's `pages`, as the API gives it),
//     // resolving its instances and token references against `system`: the
//     // version's token tree with its components under `$components`, the
//     // design system format of omni-render's `DesignSystem::from_json`.
//     show(page: unknown, system: unknown): void;
//     // The shown page's boards, each without its children (`Scene::layers`,
//     // crates/omni-render/tests/layers.rs).
//     layers(): Layer[];
//     // The node at `address` with every node below it, as the renderer
//     // resolved it (`Scene::layers_of`): a node inside an instance is
//     // addressed `<instanceId>/<componentNodeId>`, a board not read yet has no
//     // children; `null` when the page shown has no such node.
//     layersOf(address: string): Layer | null;
//     // The board that holds the node at `address` (`Scene::board_of`).
//     boardOf(address: string): string | null;
//     // Selects the node at `address` on the canvas (`null` clears it).
//     select(address: string | null): void;
//     // Calls `listener` when the reviewer selects a node on the canvas;
//     // returns the function that stops it.
//     onSelect(listener: (address: string | null) => void): () => void;
//     destroy(): void;
//   };
//   export function createDesignRenderer(canvas: HTMLCanvasElement): Promise<DesignRenderer>;
//
// The page shows:
//
// - the doc's title as a heading, and a `<canvas>` that it gives the renderer;
// - a page list: a navigation or region named "Pages" with a button per page
//   of the version, in order, named by the page's name. The page shown is
//   marked with `aria-current` (or `aria-pressed` / `aria-selected` "true");
//   the first page is shown at first, and a click on another page shows that
//   one;
// - a layers panel (US-126): a region named "Layers" holding an element of
//   role "tree", with one "treeitem" per board of the page shown, folded. A
//   board's treeitem holds a button with `aria-expanded` that unfolds it,
//   listing its layers below it, every level, nodes inside instances
//   included. A treeitem's text starts with the node's name, it has
//   `aria-level` (1 for a board), and the selected node's treeitem has
//   `aria-selected="true"` (no other has). Clicking a treeitem selects that
//   node on the canvas, and a node selected on the canvas opens its board and
//   is the selected treeitem, scrolled into view.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { answerDesignRead } from "./testing/design-boards";

type Layer = { address: string; name: string; type: string; children: Layer[] };
type Shown = { page: unknown; system: unknown; board?: string | null };

/** What the fake renderers were asked to do. */
const fake = vi.hoisted(() => {
  type FakeLayer = { address: string; name: string; type: string; children: FakeLayer[] };
  type StandIn = {
    family: string;
    weight: number;
    style: string;
    drawnAs: { family: string; weight: number; style: string };
    texts: number;
  };
  type Instance = {
    canvas: HTMLCanvasElement;
    shown: { page: unknown; system: unknown; board?: string | null }[];
    selected: (string | null)[];
    revealed: (string | null)[];
    modeCalls: (Record<string, string> | string)[];
    listeners: Set<(address: string | null) => void>;
    destroyed: boolean;
  };
  return {
    instances: [] as Instance[],
    /** The layers the renderer gives for each page, by the page's ID. */
    layersByPage: new Map<string, FakeLayer[]>(),
    /** Whether the renderer puts boards that arrive in place (`updateBoards`) instead of being shown the page again. */
    incremental: false,
    /** The fonts that stand in on each board, by the board's ID, as `fontsStoodIn` lists them. */
    standInsByBoard: new Map<string, StandIn[]>(),
    /** The token tree the head's design system has, instead of `TOKENS`. */
    tokens: null as Record<string, unknown> | null,
  };
});

vi.mock("@/design/renderer", () => ({
  createDesignRenderer: async (canvas: HTMLCanvasElement) => {
    const instance = {
      canvas,
      shown: [] as Shown[],
      selected: [] as (string | null)[],
      revealed: [] as (string | null)[],
      modeCalls: [] as (Record<string, string> | string)[],
      listeners: new Set<(address: string | null) => void>(),
      destroyed: false,
    };
    fake.instances.push(instance);
    type FakeLayer = { address: string; name: string; type: string; children: FakeLayer[] };
    type ShownPage = { id?: string; children?: { id: string; children?: unknown[] }[] };
    // The page as the renderer has it: the page shown last, with the boards put in place since.
    let current: ShownPage | undefined;
    const lastPage = () => current;
    const boards = (): FakeLayer[] => {
      const id = lastPage()?.id;
      return id ? (fake.layersByPage.get(id) ?? []) : [];
    };
    const find = (layers: FakeLayer[], address: string): FakeLayer | null => {
      for (const layer of layers) {
        if (layer.address === address) return layer;
        const below = find(layer.children, address);
        if (below) return below;
      }
      return null;
    };
    // A board of the page shown that is read, not a placeholder without its children.
    const isRead = (board: string) =>
      lastPage()?.children?.some((entry) => entry.id === board && entry.children !== undefined) ?? false;
    // The node at `address` as the renderer has it: a placeholder board without its children, no node below it.
    const nodeAt = (address: string): { board: FakeLayer; layer: FakeLayer } | null => {
      const board = boards().find((candidate) => find([candidate], address));
      if (!board) return null;
      if (board.address === address) return { board, layer: isRead(board.address) ? board : { ...board, children: [] } };
      return isRead(board.address) ? { board, layer: find([board], address)! } : null;
    };
    return {
      show(page: unknown, system: unknown, board?: string | null) {
        instance.shown.push({ page, system, board });
        current = page as ShownPage;
      },
      ...(fake.incremental
        ? {
            updateBoards(boards: { id: string }[]) {
              if (!current) return false;
              const arrived = new Map(boards.map((board) => [board.id, board]));
              current = { ...current, children: current.children?.map((board) => arrived.get(board.id) ?? board) };
              return true;
            },
            stepBoards() {
              return false;
            },
          }
        : {}),
      layers() {
        return boards().map((board) => ({ ...board, children: [] }));
      },
      // The stand-ins of the boards read, once per face asked for with the texts of all of them.
      fontsStoodIn() {
        const listed: { family: string; weight: number; style: string; drawnAs: unknown; texts: number }[] = [];
        for (const board of lastPage()?.children ?? []) {
          if (!isRead(board.id)) continue;
          for (const entry of fake.standInsByBoard.get(board.id) ?? []) {
            const same = listed.find(
              (seen) => seen.family === entry.family && seen.weight === entry.weight && seen.style === entry.style,
            );
            if (same) same.texts += entry.texts;
            else listed.push({ ...entry });
          }
        }
        return listed;
      },
      layersOf(address: string) {
        return nodeAt(address)?.layer ?? null;
      },
      boardOf(address: string) {
        return nodeAt(address)?.board.address ?? null;
      },
      select(address: string | null, reveal?: boolean) {
        instance.selected.push(address);
        if (reveal) instance.revealed.push(address);
      },
      onSelect(listener: (address: string | null) => void) {
        instance.listeners.add(listener);
        return () => instance.listeners.delete(listener);
      },
      // US-044's inspect panel (src/design-inspect.test.tsx); this test only
      // records the modes it is given.
      setMode(choice: Record<string, string> | string) {
        instance.modeCalls.push(choice);
      },
      inspect() {
        return null;
      },
      // US-045's comment pins (src/design-review.test.tsx): a box for every node the renderer has.
      boxOf(address: string) {
        const at = nodeAt(address);
        return at ? { x: 0, y: 0, width: 10, height: 10, name: at.layer.name } : null;
      },
      onView() {
        return () => {};
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
}));

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const DOC_ID = "0192d7a0-7777-7000-8000-000000000001";
const DESIGN_SYSTEM_ID = "0192d7a0-7777-7000-8000-000000000002";
const V1 = "0192d7a0-7777-7000-8000-000000000021";
const DS_V1 = "0192d7a0-7777-7000-8000-000000000022";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const DOC_TITLE = "Checkout screens";
const DOC_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/docs/${DOC_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T10:00:00Z";
const AUTHOR = { run: "run-1", role: "designer", model: "test-model", harness: "test-harness" };

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

const fixed = (value: number) => ({ type: "fixed", value });

const PAGES = [
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
                content: "Total 96 kr",
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

const COMPONENTS = [
  {
    id: CART,
    type: "component",
    name: "Cart",
    width: fixed(20),
    height: fixed(20),
    layout: { mode: "none" },
    children: [{ id: WHEEL, type: "frame", name: "Wheel", width: fixed(6), height: fixed(6), layout: { mode: "none" } }],
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
      {
        id: LABEL,
        type: "text",
        name: "Label",
        content: "Buy",
        fontFamily: "Inter",
        fontSize: 16,
        lineHeight: 1.25,
      },
      { id: ICON, type: "instance", name: "Icon", component: CART },
    ],
  },
];

const TOKENS = {
  $extensions: { omnistate: { modes: ["light", "dark"] } },
  color: {
    $type: "color",
    brand: {
      $value: "#ff0000",
      $extensions: { omnistate: { modes: { light: "#ff0000", dark: "#0000ff" } } },
    },
  },
};

function layer(address: string, name: string, type: string, children: Layer[] = []): Layer {
  return { address, name, type, children };
}

/** The layers the renderer resolves for each page: Button's layers sit under the instance Buy. */
function seedLayers() {
  fake.layersByPage.set(MOBILE, [
    layer(CART_BOARD, "Cart", "board", [
      layer(BUY, "Buy", "instance", [
        layer(`${BUY}/${LABEL}`, "Label", "text"),
        layer(`${BUY}/${ICON}`, "Icon", "instance", [layer(`${BUY}/${WHEEL}`, "Wheel", "frame")]),
      ]),
      layer(SUMMARY, "Summary", "frame", [layer(TOTAL, "Total", "text")]),
    ]),
    layer(RECEIPT, "Receipt", "board"),
  ]);
  fake.layersByPage.set(DESKTOP, [layer(WIDE_CART, "Wide cart", "board", [layer(HEADING, "Heading", "text")])]);
}

type Call = { method: string; path: string };

/**
 * A fake of the API: the auth routes, the project, the design doc and its
 * head, `pages`, which the test may change. A read of the board `held` waits
 * until `release` is called.
 */
function fakeApi(pages: unknown[] = PAGES, held: string | null = null) {
  let release: () => void = () => {};
  const gate = held ? new Promise<void>((resolve) => (release = resolve)) : Promise.resolve();
  const api = { calls: [] as Call[], pages, release: () => release() };
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
    mainline_version_id: V1,
    drafts: [],
    claims: [],
  };
  const designSystem = {
    id: DESIGN_SYSTEM_ID,
    project_id: PROJECT_ID,
    kind: "design-system",
    pillar: "ui",
    title: "Design system",
    created_at: AT,
    mainline_version_id: DS_V1,
    drafts: [],
    claims: [],
  };
  const item = {
    id: V1,
    doc_id: DOC_ID,
    draft_id: "0192d7a0-7777-7000-8000-000000000031",
    removed: false,
    created_at: AT,
    agent: AUTHOR,
  };
  const manifest = {
    pages: (PAGES as { id: string; children: { id: string }[] }[]).map((page) => ({
      id: page.id,
      chunk: "0".repeat(64),
      boards: page.children.map((board) => ({ id: board.id, chunk: "1".repeat(64) })),
    })),
  };
  const version = {
    ...item,
    design_system_version_id: DS_V1,
    manifest,
    nodes: [],
    tags: "<Page name=\"Mobile\"/>",
    pages: PAGES as unknown[],
    components: COMPONENTS,
    tokens: fake.tokens ?? TOKENS,
  };

  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(raw, "http://localhost:3000");
    const path = url.pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    api.calls.push({ method, path });
    if (held && path.endsWith(`/boards/${held}`)) await gate;

    // US-058: the head's outline and its boards, one by one.
    const read = answerDesignRead(API, method, url, (id) => (id === DOC_ID ? { ...version, pages: api.pages } : null));
    if (read) return json(read.status, read.body);
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
          docs: [designSystem, doc],
          threads: [],
          waiting: { comments: [] },
        });
      case `GET ${API}/docs/${DOC_ID}`:
        return json(200, { doc });
      case `GET ${API}/docs/${DOC_ID}/versions`:
        return json(200, { doc_id: DOC_ID, versions: [item] });
      case `GET ${API}/docs/${DOC_ID}/html`:
        // A design version has no HTML.
        return json(200, { doc_id: DOC_ID, version_id: V1, html: "" });
      case `GET ${API}/comments`:
        return json(200, { project_id: PROJECT_ID, comments: [] });
      case `GET ${API}/versions/${V1}`:
        return json(200, { version });
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

/** Opens the live socket, confirms the page's subscription to the project, and returns a sender of `doc.changed`. */
async function openSocket() {
  await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
  const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
  socket.open();
  await waitFor(() =>
    expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })),
  );
  socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 40 });
  let seq = 40;
  return () => {
    seq += 1;
    socket.receive({ type: "event", seq, event: "doc.changed", project_id: PROJECT_ID, at: AT, data: { doc_id: DOC_ID, agent: AUTHOR } });
  };
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

/** The renderer the page made last, once it has one for a canvas on the page. */
async function findRenderer() {
  return waitFor(() => {
    const latest = fake.instances[fake.instances.length - 1];
    expect(latest, "the page made a renderer for its canvas (createDesignRenderer)").toBeTruthy();
    expect(latest.canvas, "the renderer was given a canvas").toBeInstanceOf(HTMLCanvasElement);
    expect(document.body.contains(latest.canvas), "the renderer's canvas is on the page").toBe(true);
    return latest;
  });
}

/** Waits until the renderer's last shown page is `pageId`, and returns what it was given. */
async function findShown(pageId: string): Promise<Shown> {
  const renderer = await findRenderer();
  return waitFor(() => {
    const last = renderer.shown[renderer.shown.length - 1];
    expect(last, "the page showed a page of the design on the renderer").toBeTruthy();
    expect((last.page as { id?: string }).id).toBe(pageId);
    return last;
  });
}

/**
 * Waits until the renderer's last shown page is `page` with all its boards
 * loaded: the page shows its boards as placeholders until it has read them
 * (US-058, src/design-boards.test.tsx).
 */
async function findShownWhole(page: unknown): Promise<Shown> {
  const renderer = await findRenderer();
  return waitFor(() => {
    const last = renderer.shown[renderer.shown.length - 1];
    expect(last, "the page showed a page of the design on the renderer").toBeTruthy();
    expect(last.page, "the page with every board loaded").toEqual(page);
    return last;
  });
}

function pagesPanel() {
  return (
    screen.queryByRole("navigation", { name: /^pages$/i }) ?? screen.getByRole("region", { name: /^pages$/i })
  );
}

/** The page list, once it lists the pages. */
async function findPagesPanel() {
  return waitFor(() => {
    const panel = pagesPanel();
    expect(within(panel).getAllByRole("button").length, "the page list has a button per page").toBeGreaterThan(0);
    return panel;
  });
}

function isCurrent(element: HTMLElement) {
  const current = element.getAttribute("aria-current");
  return (
    (current !== null && current !== "false") ||
    element.getAttribute("aria-pressed") === "true" ||
    element.getAttribute("aria-selected") === "true"
  );
}

function pageButton(name: string) {
  return within(pagesPanel()).getByRole("button", { name: new RegExp(`^${name}\\b`) });
}

async function findLayersTree() {
  return waitFor(() => {
    const region = screen.getByRole("region", { name: /^layers$/i });
    return within(region).getByRole("tree");
  });
}

function queryTreeItem(name: string) {
  const region = screen.queryByRole("region", { name: /^layers$/i });
  const items = region ? within(region).queryAllByRole("treeitem") : [];
  return items.find((item) => new RegExp(`^${name}\\b`).test((item.textContent ?? "").trim())) ?? null;
}

/** The treeitem of the node named `name`, in the layers panel. */
function treeItem(name: string) {
  const item = queryTreeItem(name);
  expect(item, `the layers panel lists ${name}`).toBeTruthy();
  return item!;
}

/** The button that folds and unfolds the board named `name`. */
function chevron(name: string) {
  return within(treeItem(name)).getByRole("button");
}

/** The rows of the layers panel, as `<level>:<first word of the name>`. */
function rowNames(filter: (item: HTMLElement) => boolean = () => true) {
  const region = screen.getByRole("region", { name: /^layers$/i });
  return within(region)
    .queryAllByRole("treeitem")
    .filter(filter)
    .map((item) => item.getAttribute("aria-level") + ":" + (item.textContent ?? "").trim().split(/\s+/)[0]);
}

function selectedNames() {
  return rowNames((item) => item.getAttribute("aria-selected") === "true");
}

/** The reviewer selects `address` on the canvas. */
function selectOnCanvas(address: string | null) {
  const renderer = fake.instances[fake.instances.length - 1];
  act(() => {
    for (const listener of Array.from(renderer.listeners)) listener(address);
  });
}

let api: ReturnType<typeof fakeApi>;
const scrollIntoView = Element.prototype.scrollIntoView;

beforeEach(() => {
  fake.instances.length = 0;
  fake.layersByPage.clear();
  fake.standInsByBoard.clear();
  fake.tokens = null;
  seedLayers();
  api = fakeApi();
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  Element.prototype.scrollIntoView = scrollIntoView;
  vi.useRealTimers();
  cleanup();
  vi.unstubAllGlobals();
});

// The canvas -----------------------------------------------------------------------------

describe("the design on the renderer's canvas", () => {
  test("the first page of the head is shown, resolved against the design system", async () => {
    renderAt(DOC_PATH);
    const shown = await findShownWhole(PAGES[0]);
    expect(shown.page, "the page as the head holds it, with its boards").toEqual(PAGES[0]);
    expect(
      api.calls.some((call) => call.method === "GET" && call.path.startsWith(`${API}/versions/`)),
      "the page reads no version",
    ).toBe(false);
    const system = shown.system as Record<string, unknown>;
    expect(system, "the design system: the version's tokens with its components").toBeTruthy();
    expect(system.$components, "the components under $components").toEqual(COMPONENTS);
    expect(system.color, "the version's token tree").toEqual(TOKENS.color);
    expect(system.$extensions, "the token tree's modes").toEqual(TOKENS.$extensions);
  });
});

// The mode selects ------------------------------------------------------------------------

describe("the mode selects", () => {
  const select = (name: string) => screen.getByLabelText(name) as HTMLSelectElement;

  test("a collection with its own modes has a select of its own beside the shared Mode select, and choosing there sets that collection only", async () => {
    fake.tokens = {
      $extensions: { omnistate: { modes: ["light", "dark"] } },
      color: { $type: "color", surface: { $value: "#ffffff" } },
      density: {
        $extensions: { omnistate: { modes: ["comfortable", "compact"] } },
        space: { md: { $type: "dimension", $value: "16px" } },
      },
    };
    api = fakeApi();
    renderAt(DOC_PATH);
    const renderer = await findRenderer();
    await screen.findByLabelText("density");
    expect(Array.from(select("Mode").options).map((option) => option.value)).toEqual(["light", "dark"]);
    expect(Array.from(select("density").options).map((option) => option.value)).toEqual(["comfortable", "compact"]);

    fireEvent.change(select("density"), { target: { value: "compact" } });
    expect(renderer.modeCalls.at(-1)).toEqual({ color: "light", density: "compact" });
    fireEvent.change(select("Mode"), { target: { value: "dark" } });
    expect(renderer.modeCalls.at(-1)).toEqual({ color: "dark", density: "compact" });
  });

  test("when every collection has its own modes the root list has no select", async () => {
    fake.tokens = {
      $extensions: { omnistate: { modes: ["light", "dark"] } },
      color: {
        $extensions: { omnistate: { modes: ["light", "dark"] } },
        surface: { $type: "color", $value: "#ffffff" },
      },
      density: {
        $extensions: { omnistate: { modes: ["comfortable", "compact"] } },
        space: { md: { $type: "dimension", $value: "16px" } },
      },
    };
    api = fakeApi();
    renderAt(DOC_PATH);
    await screen.findByLabelText("density");
    expect(screen.getByLabelText("color")).toBeTruthy();
    expect(screen.queryByLabelText("Mode")).toBeNull();
  });
});

// The page list --------------------------------------------------------------------------

describe("the page list", () => {
  test("choosing a page shows it on the canvas and marks it", async () => {
    renderAt(DOC_PATH);
    await findShown(MOBILE);
    await findPagesPanel();
    fireEvent.click(pageButton("Desktop"));
    const shown = await findShownWhole(PAGES[1]);
    expect(shown.page).toEqual(PAGES[1]);
    await waitFor(() => expect(isCurrent(pageButton("Desktop"))).toBe(true));
    expect(isCurrent(pageButton("Mobile"))).toBe(false);

    fireEvent.click(pageButton("Mobile"));
    expect((await findShownWhole(PAGES[0])).page).toEqual(PAGES[0]);
  });

  test("choosing a page lists that page's boards", async () => {
    renderAt(DOC_PATH);
    await findLayersTree();
    await waitFor(() => expect(rowNames()).toEqual(["1:Cart", "1:Receipt"]));
    await findPagesPanel();
    fireEvent.click(pageButton("Desktop"));
    await waitFor(() => expect(rowNames(), "the boards of the other page are gone").toEqual(["1:Wide"]));
  });
});

// The layers panel -----------------------------------------------------------------------

describe("the layers panel", () => {
  test("a board's chevron lists its layers below it at their depth, in order, nodes inside instances under their instance, and folds them again", async () => {
    renderAt(DOC_PATH);
    await findShownWhole(PAGES[0]);
    await waitFor(() => expect(rowNames()).toEqual(["1:Cart", "1:Receipt"]));

    fireEvent.click(chevron("Cart"));
    expect(rowNames()).toEqual(["1:Cart", "2:Buy", "3:Label", "3:Icon", "4:Wheel", "2:Summary", "3:Total", "1:Receipt"]);
    expect(chevron("Cart").getAttribute("aria-expanded")).toBe("true");

    fireEvent.click(chevron("Cart"));
    expect(rowNames()).toEqual(["1:Cart", "1:Receipt"]);
    expect(chevron("Cart").getAttribute("aria-expanded")).toBe("false");
  });

  test("a click on a board's row selects the board and leaves it folded", async () => {
    renderAt(DOC_PATH);
    const renderer = await findRenderer();
    await findShownWhole(PAGES[0]);
    await waitFor(() => expect(rowNames()).toEqual(["1:Cart", "1:Receipt"]));

    fireEvent.click(treeItem("Cart"));
    await waitFor(() => expect(selectedNames()).toEqual(["1:Cart"]));
    expect(renderer.selected[renderer.selected.length - 1]).toBe(CART_BOARD);
    expect(rowNames()).toEqual(["1:Cart", "1:Receipt"]);
    expect(chevron("Cart").getAttribute("aria-expanded")).toBe("false");
  });

  test("an open board stays open when the page is shown again, with the layers the board has now", async () => {
    renderAt(DOC_PATH);
    const send = await openSocket();
    await findShownWhole(PAGES[0]);
    await waitFor(() => expect(rowNames()).toEqual(["1:Cart", "1:Receipt"]));
    fireEvent.click(chevron("Cart"));

    // An agent deletes Summary, and Total with it.
    const cart = PAGES[0].children[0];
    const changed = [{ ...PAGES[0], children: [{ ...cart, children: [cart.children![0]] }, PAGES[0].children[1]] }, PAGES[1]];
    api.pages = changed;
    fake.layersByPage.set(MOBILE, [
      layer(CART_BOARD, "Cart", "board", [
        layer(BUY, "Buy", "instance", [
          layer(`${BUY}/${LABEL}`, "Label", "text"),
          layer(`${BUY}/${ICON}`, "Icon", "instance", [layer(`${BUY}/${WHEEL}`, "Wheel", "frame")]),
        ]),
      ]),
      layer(RECEIPT, "Receipt", "board"),
    ]);
    send();
    await findShownWhole(changed[0]);
    await waitFor(() => expect(rowNames()).toEqual(["1:Cart", "2:Buy", "3:Label", "3:Icon", "4:Wheel", "1:Receipt"]));
    expect(chevron("Cart").getAttribute("aria-expanded")).toBe("true");
  });
});

// Boards put in place as they arrive ----------------------------------------------------

/** Animation frames the page asked for, run by the test. */
function stubFrames() {
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => frames.push(callback));
  vi.stubGlobal("cancelAnimationFrame", () => {});
  return {
    /** Runs the frames asked for so far, once one is asked for. */
    async run() {
      await waitFor(() => expect(frames.length, "the page puts a board in place in the next frame").toBeGreaterThan(0));
      act(() => {
        for (const frame of frames.splice(0)) frame(performance.now());
      });
    },
  };
}

describe("a board that arrives while the page is shown", () => {
  beforeEach(() => {
    fake.incremental = true;
  });
  afterEach(() => {
    fake.incremental = false;
  });

  test("expanding a board that is not read lists no rows and does not read it; once it is put in place its layers are listed in that frame", async () => {
    const frames = stubFrames();
    api = fakeApi(PAGES, CART_BOARD);
    renderAt(DOC_PATH);
    await findShown(MOBILE);
    await waitFor(() => expect(rowNames()).toEqual(["1:Cart", "1:Receipt"]));
    const reads = () => api.calls.filter((call) => call.path.endsWith(`/boards/${CART_BOARD}`));
    await waitFor(() => expect(reads()).toHaveLength(1));

    fireEvent.click(chevron("Cart"));
    expect(chevron("Cart").getAttribute("aria-expanded")).toBe("true");
    expect(rowNames(), "Cart is not read, so it has no rows yet").toEqual(["1:Cart", "1:Receipt"]);
    expect(reads(), "expanding it does not read it again").toHaveLength(1);

    api.release();
    await frames.run();
    expect(rowNames()).toEqual(["1:Cart", "2:Buy", "3:Label", "3:Icon", "4:Wheel", "2:Summary", "3:Total", "1:Receipt"]);
  });

  test("a node a link names on a board that is put in place is selected in that frame, its board open", async () => {
    const frames = stubFrames();
    api = fakeApi(PAGES, CART_BOARD);
    renderAt(`${DOC_PATH}?node=${TOTAL}`);
    await findShown(MOBILE);
    await waitFor(() => expect(rowNames()).toEqual(["1:Cart", "1:Receipt"]));

    api.release();
    await frames.run();
    expect(selectedNames()).toEqual(["3:Total"]);
    expect(chevron("Cart").getAttribute("aria-expanded")).toBe("true");
  });
});

describe("fonts that stood in", () => {
  beforeEach(() => {
    fake.incremental = true;
  });
  afterEach(() => {
    fake.incremental = false;
  });

  test("the error line has a sentence per font that stood in, and a board put in place later adds one or updates a count", async () => {
    const roboto = { family: "Roboto", weight: 700, style: "normal", drawnAs: { family: "Inter", weight: 400, style: "normal" } };
    const geist = { family: "Geist", weight: 400, style: "italic", drawnAs: { family: "Geist", weight: 400, style: "normal" } };
    fake.standInsByBoard.set(CART_BOARD, [{ ...roboto, texts: 1 }]);
    fake.standInsByBoard.set(RECEIPT, [{ ...roboto, texts: 2 }, { ...geist, texts: 1 }]);
    // Receipt is given an empty list of children, so the fake counts it as read once it arrives.
    const [cart, receipt] = PAGES[0].children;
    const pages = [{ ...PAGES[0], children: [cart, { ...receipt, children: [] }] }, PAGES[1]];
    const frames = stubFrames();
    api = fakeApi(pages, RECEIPT);
    renderAt(DOC_PATH);
    await frames.run();
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toBe(
        "1 text in Roboto 700 is drawn in Inter 400: the design system has no font file for it",
      ),
    );

    api.release();
    await frames.run();
    await waitFor(() =>
      expect(screen.getByRole("alert").textContent).toBe(
        "3 texts in Roboto 700 are drawn in Inter 400: the design system has no font file for it " +
          "1 text in Geist 400 italic is drawn in Geist 400: the design system has no font file for it",
      ),
    );
  });
});

// Selection in step ----------------------------------------------------------------------

describe("selection between the canvas and the layers panel", () => {
  test("selecting a layer in the panel selects it on the canvas", async () => {
    renderAt(DOC_PATH);
    const renderer = await findRenderer();
    await findShownWhole(PAGES[0]);
    await waitFor(() => expect(rowNames()).toEqual(["1:Cart", "1:Receipt"]));
    fireEvent.click(chevron("Cart"));

    fireEvent.click(treeItem("Label"));
    await waitFor(() =>
      expect(renderer.selected[renderer.selected.length - 1], "the canvas selects the node inside the instance").toBe(
        `${BUY}/${LABEL}`,
      ),
    );
    await waitFor(() => expect(treeItem("Label").getAttribute("aria-selected")).toBe("true"));
    expect(selectedNames(), "one layer is selected").toEqual(["3:Label"]);

    fireEvent.click(treeItem("Summary"));
    await waitFor(() => expect(renderer.selected[renderer.selected.length - 1]).toBe(SUMMARY));
    await waitFor(() => expect(selectedNames()).toEqual(["2:Summary"]));
  });

  test("selecting a node on the canvas opens its board, selects its layer in the panel and scrolls it into view", async () => {
    const scrolled: (string | undefined)[] = [];
    Element.prototype.scrollIntoView = vi.fn(function (this: HTMLElement) {
      scrolled.push(this.dataset.address);
    });
    renderAt(DOC_PATH);
    await findRenderer();
    await findShownWhole(PAGES[0]);
    await waitFor(() => expect(rowNames()).toEqual(["1:Cart", "1:Receipt"]));
    await waitFor(() => expect(fake.instances[fake.instances.length - 1].listeners.size).toBeGreaterThan(0));

    selectOnCanvas(`${BUY}/${WHEEL}`);
    await waitFor(() => expect(selectedNames(), "the Wheel inside the instance").toEqual(["4:Wheel"]));
    expect(chevron("Cart").getAttribute("aria-expanded")).toBe("true");
    expect(scrolled).toEqual([`${BUY}/${WHEEL}`]);

    selectOnCanvas(TOTAL);
    await waitFor(() => expect(selectedNames()).toEqual(["3:Total"]));

    selectOnCanvas(null);
    await waitFor(() => expect(selectedNames(), "a click on no board clears the selection").toEqual([]));
  });
});

// Links to a place in the design ---------------------------------------------------------

/** A page of 200 boards, B0 to B199, each holding one text, T0 to T199. */
const GRID = "pg000003";
const GRID_BOARDS = Array.from({ length: 200 }, (_, i) => ({
  id: `b${String(i).padStart(7, "0")}`,
  type: "board",
  name: `B${i}`,
  x: (i % 20) * 120,
  y: Math.floor(i / 20) * 120,
  width: fixed(100),
  height: fixed(100),
  layout: { mode: "none" },
  children: [{ id: `t${String(i).padStart(7, "0")}`, type: "text", name: `T${i}`, content: `T${i}`, fontFamily: "Inter", fontSize: 14, lineHeight: 1.25 }],
}));
const GRID_PAGE = { id: GRID, type: "page", name: "Grid", children: GRID_BOARDS };
const B199 = "b0000199";
const T199 = "t0000199";

function seedGridLayers() {
  fake.layersByPage.set(
    GRID,
    GRID_BOARDS.map((board) => layer(board.id, board.name, "board", [layer(board.children[0].id, board.children[0].name, "text")])),
  );
}

describe("a link to a place in the design", () => {
  test("?node alone opens the page whose board holds the node, however many boards come before it", async () => {
    api = fakeApi([...PAGES, GRID_PAGE]);
    seedGridLayers();
    const router = renderAt(`${DOC_PATH}?node=${T199}`);
    await findShown(GRID);
    await waitFor(() => expect(selectedNames()).toEqual(["2:T199"]), { timeout: 5000 });
    await waitFor(() => expect(router.state.location.search).toEqual({ board: B199, node: T199 }));
    expect((await findRenderer()).revealed, "a link moves the view to the node it names").toEqual([T199]);
  });

  test("?board alone opens the page that holds the board with the view on it, keeps it in the address and selects nothing", async () => {
    api = fakeApi([...PAGES, GRID_PAGE]);
    seedGridLayers();
    const router = renderAt(`${DOC_PATH}?board=${B199}`);
    const shown = await findShown(GRID);
    expect(shown.board).toBe(B199);
    await waitFor(() => expect(treeItem("B199")).toBeTruthy());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(router.state.location.search).toEqual({ board: B199 });
    expect(selectedNames()).toEqual([]);
  });

  test("a board and a node that the design does not have open the first page with nothing selected", async () => {
    const router = renderAt(`${DOC_PATH}?board=zzzzzzzz&node=yyyyyyyy`);
    await findShownWhole(PAGES[0]);
    await waitFor(() => expect(router.state.location.search).toEqual({ board: CART_BOARD }));
    expect(selectedNames()).toEqual([]);
  });
});

// Agents writing while the reviewer looks ----------------------------------------------

/** Waits, without the clock, until the page has set a timer. */
async function untilTimerPending() {
  for (let tick = 0; vi.getTimerCount() === 0 && tick < 1000; tick += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  expect(vi.getTimerCount(), "the page is about to show the board that arrived").toBeGreaterThan(0);
}

describe("the design as agents change it", () => {
  test("a node selected on the canvas stays selected when a board arrives at that moment", async () => {
    api = fakeApi(PAGES, RECEIPT);
    renderAt(DOC_PATH);
    const renderer = await findRenderer();
    await waitFor(() => {
      const cart = ((renderer.shown[renderer.shown.length - 1].page as { children: { id: string; children?: unknown[] }[] }).children)[0];
      expect(cart.children, "Cart is loaded and shown").toBeTruthy();
    });

    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    api.release();
    await untilTimerPending();
    // The reviewer clicks Total just before the page shows Receipt.
    act(() => {
      for (const listener of Array.from(renderer.listeners)) listener(TOTAL);
      vi.runOnlyPendingTimers();
    });
    vi.useRealTimers();

    await findShownWhole(PAGES[0]);
    await waitFor(() => expect(selectedNames()).toEqual(["3:Total"]));
    expect(renderer.selected[renderer.selected.length - 1]).toBe(TOTAL);
  });

  test("when an agent changes a board on a page of many boards, that board is read again and an unchanged one is not", async () => {
    api = fakeApi([...PAGES, GRID_PAGE]);
    seedGridLayers();
    // The search for T199 reads every board of the design, so all 200 are drawn.
    renderAt(`${DOC_PATH}?node=${T199}`);
    const send = await openSocket();
    await findShownWhole(GRID_PAGE);
    const reads = (board: string) => api.calls.filter((call) => call.path.endsWith(`/boards/${board}`));
    expect(reads(B199)).toHaveLength(1);

    // An agent changes the text on B199.
    const changedB199 = { ...GRID_BOARDS[199], children: [{ ...GRID_BOARDS[199].children[0], content: "Changed" }] };
    const changedGrid = { ...GRID_PAGE, children: [...GRID_BOARDS.slice(0, 199), changedB199] };
    api.pages = [...PAGES, changedGrid];
    send();
    await findShownWhole(changedGrid);
    expect(reads(B199), "the changed board is read again").toHaveLength(2);
    expect(reads("b0000150"), "an unchanged board is not").toHaveLength(1);
  });

  test("when an agent removes the selected node, the selection is cleared and the view stays", async () => {
    renderAt(DOC_PATH);
    const send = await openSocket();
    const renderer = await findRenderer();
    await findShownWhole(PAGES[0]);
    await waitFor(() => expect(renderer.listeners.size).toBeGreaterThan(0));
    selectOnCanvas(TOTAL);
    await waitFor(() => expect(selectedNames()).toEqual(["3:Total"]));

    // An agent deletes Summary, and Total with it.
    const cart = PAGES[0].children[0];
    const changed = [{ ...PAGES[0], children: [{ ...cart, children: [cart.children![0]] }, PAGES[0].children[1]] }, PAGES[1]];
    api.pages = changed;
    fake.layersByPage.set(MOBILE, [layer(CART_BOARD, "Cart", "board", [layer(BUY, "Buy", "instance")]), layer(RECEIPT, "Receipt", "board")]);
    send();
    await findShownWhole(changed[0]);
    await waitFor(() => expect(selectedNames()).toEqual([]));
    expect(await screen.findByText(/select a node/i)).toBeTruthy();
    expect(fake.instances.filter((instance) => !instance.destroyed), "the same renderer, so the zoom and pan stay").toEqual([renderer]);
  });

  test("when an agent deletes the open design, the page shows it as not found", async () => {
    renderAt(DOC_PATH);
    await openSocket();
    const renderer = await findRenderer();
    await findShownWhole(PAGES[0]);

    const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
    socket.receive({ type: "event", seq: 41, event: "doc.deleted", project_id: PROJECT_ID, at: AT, data: { doc_id: DOC_ID } });
    await screen.findByText("The doc could not be loaded. Try again in a moment.");
    expect(document.body.contains(renderer.canvas), "the deleted design is no longer drawn").toBe(false);
  });
});
