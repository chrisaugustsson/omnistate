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
// The page loads the doc (GET .../docs/<doc>) and its mainline version by ID
// (GET .../versions/<mainline_version_id>), which for a design version holds
// `pages` (the pages as JSON nodes, each with its boards), `components` (the
// components its instances use, from the design system version it recorded)
// and `tokens` (that design system version's whole DTCG token tree,
// crates/omni-api/tests/components.rs).
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
//     // The shown page's boards with every node below them, as the renderer
//     // resolved them (`Scene::layers`, crates/omni-render/tests/layers.rs):
//     // a node inside an instance is addressed `<instanceId>/<componentNodeId>`.
//     layers(): Layer[];
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
// - a layers panel: a region named "Layers" holding an element of role
//   "tree", with one "treeitem" per layer of the page shown, every level
//   listed, nodes inside instances included. A treeitem's accessible name
//   starts with the node's name, it has `aria-level` (1 for a board), and the
//   selected node's treeitem has `aria-selected="true"` (no other has).
//   Clicking a treeitem selects that node on the canvas, and a node selected
//   on the canvas is the selected treeitem.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

type Layer = { address: string; name: string; type: string; children: Layer[] };
type Shown = { page: unknown; system: unknown };

/** What the fake renderers were asked to do. */
const fake = vi.hoisted(() => {
  type FakeLayer = { address: string; name: string; type: string; children: FakeLayer[] };
  type Instance = {
    canvas: HTMLCanvasElement;
    shown: { page: unknown; system: unknown }[];
    selected: (string | null)[];
    listeners: Set<(address: string | null) => void>;
    destroyed: boolean;
  };
  return {
    instances: [] as Instance[],
    /** The layers the renderer gives for each page, by the page's ID. */
    layersByPage: new Map<string, FakeLayer[]>(),
  };
});

vi.mock("@/design/renderer", () => ({
  createDesignRenderer: async (canvas: HTMLCanvasElement) => {
    const instance = {
      canvas,
      shown: [] as Shown[],
      selected: [] as (string | null)[],
      listeners: new Set<(address: string | null) => void>(),
      destroyed: false,
    };
    fake.instances.push(instance);
    return {
      show(page: unknown, system: unknown) {
        instance.shown.push({ page, system });
      },
      layers() {
        const last = instance.shown[instance.shown.length - 1];
        const id = (last?.page as { id?: string } | undefined)?.id;
        return id ? (fake.layersByPage.get(id) ?? []) : [];
      },
      select(address: string | null) {
        instance.selected.push(address);
      },
      onSelect(listener: (address: string | null) => void) {
        instance.listeners.add(listener);
        return () => instance.listeners.delete(listener);
      },
      // US-044's mode switch and inspect panel (src/design-inspect.test.tsx);
      // this test does not look at them.
      setMode() {},
      inspect() {
        return null;
      },
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
const CS_FIRST = "0192d7a0-7777-7000-8000-000000000011";
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

/** A fake of the API: the auth routes, the project, the design doc and its mainline version. */
function fakeApi() {
  const api = { calls: [] as Call[] };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  const doc = {
    id: DOC_ID,
    project_id: PROJECT_ID,
    kind: "design",
    module: "ui",
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
    module: "ui",
    title: "Design system",
    created_at: AT,
    mainline_version_id: DS_V1,
    drafts: [],
    claims: [],
  };
  const item = {
    id: V1,
    doc_id: DOC_ID,
    changeset_id: CS_FIRST,
    draft_id: "0192d7a0-7777-7000-8000-000000000031",
    removed: false,
    created_at: AT,
    agent: AUTHOR,
  };
  const changesets = [
    {
      id: CS_FIRST,
      project_id: PROJECT_ID,
      title: "Design the checkout",
      description: "",
      status: "merged",
      run_tag: "run-1",
      created_at: AT,
      history: [],
      approved_versions: [],
      decisions: [],
    },
  ];
  const manifest = {
    pages: PAGES.map((page) => ({
      id: page.id,
      chunk: "0".repeat(64),
      boards: page.children.map((board) => ({ id: board.id, chunk: "1".repeat(64) })),
    })),
  };

  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(raw, "http://localhost:3000");
    const path = url.pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    api.calls.push({ method, path });

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
          changesets,
          threads: [],
          waiting: { decisions: [], changesets: [], comments: [], questions: [] },
        });
      case `GET ${API}/changesets`:
        return json(200, { changesets });
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
        return json(200, {
          version: {
            ...item,
            design_system_version_id: DS_V1,
            manifest,
            nodes: [],
            tags: "<Page name=\"Mobile\"/>",
            pages: PAGES,
            components: COMPONENTS,
            tokens: TOKENS,
          },
        });
      default:
        return json(404, { error: "not_found" });
    }
  });
  vi.stubGlobal("fetch", fetch);
  return api;
}

/** A WebSocket that connects and stays quiet. */
class FakeWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;
  readyState = FakeWebSocket.CONNECTING;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: Event) => void) | null = null;
  onclose: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  constructor(readonly url: string) {
    super();
  }

  send() {}

  close() {
    this.readyState = FakeWebSocket.CLOSED;
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

/** The treeitem of the node named `name`, in the layers panel. */
function treeItem(name: string) {
  const region = screen.getByRole("region", { name: /^layers$/i });
  return within(region).getByRole("treeitem", { name: new RegExp(`^${name}\\b`) });
}

function queryTreeItem(name: string) {
  const region = screen.queryByRole("region", { name: /^layers$/i });
  return region ? within(region).queryByRole("treeitem", { name: new RegExp(`^${name}\\b`) }) : null;
}

function selectedNames() {
  const region = screen.getByRole("region", { name: /^layers$/i });
  return within(region)
    .getAllByRole("treeitem")
    .filter((item) => item.getAttribute("aria-selected") === "true")
    .map((item) => item.getAttribute("aria-level") + ":" + (item.textContent ?? "").trim().split(/\s+/)[0]);
}

/** The reviewer selects `address` on the canvas. */
function selectOnCanvas(address: string | null) {
  const renderer = fake.instances[fake.instances.length - 1];
  act(() => {
    for (const listener of Array.from(renderer.listeners)) listener(address);
  });
}

let api: ReturnType<typeof fakeApi>;

beforeEach(() => {
  fake.instances.length = 0;
  fake.layersByPage.clear();
  seedLayers();
  api = fakeApi();
  vi.stubGlobal("WebSocket", FakeWebSocket);
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

// The canvas -----------------------------------------------------------------------------

describe("the design on the renderer's canvas", () => {
  test("a design doc's page opens on a canvas that the renderer draws, not in the HTML doc's frame", async () => {
    renderAt(DOC_PATH);
    expect(await screen.findByRole("heading", { name: DOC_TITLE })).toBeTruthy();
    await findRenderer();
    expect(document.querySelector("iframe"), "a design is no HTML doc, so it has no doc frame").toBeNull();
  });

  test("the first page of the mainline version is shown, resolved against the version's design system", async () => {
    renderAt(DOC_PATH);
    const shown = await findShown(MOBILE);
    expect(shown.page, "the page as the version holds it, with its boards").toEqual(PAGES[0]);
    expect(
      api.calls.some((call) => call.method === "GET" && call.path === `${API}/versions/${V1}`),
      "the page reads the mainline version by ID",
    ).toBe(true);
    const system = shown.system as Record<string, unknown>;
    expect(system, "the design system: the version's tokens with its components").toBeTruthy();
    expect(system.$components, "the components under $components").toEqual(COMPONENTS);
    expect(system.color, "the version's token tree").toEqual(TOKENS.color);
    expect(system.$extensions, "the token tree's modes").toEqual(TOKENS.$extensions);
  });
});

// The page list --------------------------------------------------------------------------

describe("the page list", () => {
  test("lists every page of the design in order and marks the one shown", async () => {
    renderAt(DOC_PATH);
    const panel = await findPagesPanel();
    const names = within(panel)
      .getAllByRole("button")
      .map((button) => (button.textContent ?? "").trim());
    expect(names.findIndex((name) => name.startsWith("Mobile")), "Mobile is listed").toBeGreaterThanOrEqual(0);
    expect(
      names.findIndex((name) => name.startsWith("Desktop")),
      "Desktop is listed after Mobile",
    ).toBeGreaterThan(names.findIndex((name) => name.startsWith("Mobile")));
    await waitFor(() => expect(isCurrent(pageButton("Mobile")), "Mobile is the page shown").toBe(true));
    expect(isCurrent(pageButton("Desktop"))).toBe(false);
  });

  test("choosing a page shows it on the canvas and marks it", async () => {
    renderAt(DOC_PATH);
    await findShown(MOBILE);
    await findPagesPanel();
    fireEvent.click(pageButton("Desktop"));
    const shown = await findShown(DESKTOP);
    expect(shown.page).toEqual(PAGES[1]);
    await waitFor(() => expect(isCurrent(pageButton("Desktop"))).toBe(true));
    expect(isCurrent(pageButton("Mobile"))).toBe(false);

    fireEvent.click(pageButton("Mobile"));
    expect((await findShown(MOBILE)).page).toEqual(PAGES[0]);
  });

  test("choosing a page lists that page's layers", async () => {
    renderAt(DOC_PATH);
    await findLayersTree();
    await waitFor(() => expect(treeItem("Cart")).toBeTruthy());
    await findPagesPanel();
    fireEvent.click(pageButton("Desktop"));
    await waitFor(() => expect(treeItem("Wide cart")).toBeTruthy());
    expect(queryTreeItem("Cart"), "the boards of the other page are gone").toBeNull();
    expect(queryTreeItem("Receipt")).toBeNull();
    expect(treeItem("Heading").getAttribute("aria-level")).toBe("2");
  });
});

// The layers panel -----------------------------------------------------------------------

describe("the layers panel", () => {
  test("lists the boards of the page shown and every node below them", async () => {
    renderAt(DOC_PATH);
    await findLayersTree();
    await waitFor(() => expect(treeItem("Cart")).toBeTruthy());
    const levels: [string, string][] = [
      ["Cart", "1"],
      ["Buy", "2"],
      ["Summary", "2"],
      ["Total", "3"],
      ["Receipt", "1"],
    ];
    for (const [name, level] of levels) {
      expect(treeItem(name).getAttribute("aria-level"), `${name} is at level ${level}`).toBe(level);
    }
    expect(queryTreeItem("Wide cart"), "only the page shown is listed").toBeNull();
  });

  test("lists the nodes inside instances under their instance", async () => {
    renderAt(DOC_PATH);
    await findLayersTree();
    await waitFor(() => expect(treeItem("Buy")).toBeTruthy());
    expect(treeItem("Label").getAttribute("aria-level"), "Button's Label, inside Buy").toBe("3");
    expect(treeItem("Icon").getAttribute("aria-level"), "Button's Icon, inside Buy").toBe("3");
    expect(treeItem("Wheel").getAttribute("aria-level"), "Cart's Wheel, inside Icon inside Buy").toBe("4");

    // In document order: Buy, its layers, then Summary.
    const region = screen.getByRole("region", { name: /^layers$/i });
    const order = within(region)
      .getAllByRole("treeitem")
      .map((item) => item.getAttribute("aria-level") + ":" + (item.textContent ?? "").trim().split(/\s+/)[0]);
    const at = (entry: string) => order.indexOf(entry);
    for (const entry of ["2:Buy", "3:Label", "3:Icon", "4:Wheel", "2:Summary"]) {
      expect(at(entry), `${entry} is listed: ${order.join(", ")}`).toBeGreaterThanOrEqual(0);
    }
    expect(at("2:Buy")).toBeLessThan(at("3:Label"));
    expect(at("3:Label")).toBeLessThan(at("3:Icon"));
    expect(at("3:Icon")).toBeLessThan(at("4:Wheel"));
    expect(at("4:Wheel")).toBeLessThan(at("2:Summary"));
  });
});

// Selection in step ----------------------------------------------------------------------

describe("selection between the canvas and the layers panel", () => {
  test("selecting a layer in the panel selects it on the canvas", async () => {
    renderAt(DOC_PATH);
    const renderer = await findRenderer();
    await findLayersTree();
    await waitFor(() => expect(treeItem("Label")).toBeTruthy());

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

  test("selecting a node on the canvas selects its layer in the panel", async () => {
    renderAt(DOC_PATH);
    await findRenderer();
    await findLayersTree();
    await waitFor(() => expect(treeItem("Wheel")).toBeTruthy());
    await waitFor(() => expect(fake.instances[fake.instances.length - 1].listeners.size).toBeGreaterThan(0));

    selectOnCanvas(`${BUY}/${WHEEL}`);
    await waitFor(() => expect(selectedNames(), "the Wheel inside the instance").toEqual(["4:Wheel"]));

    selectOnCanvas(TOTAL);
    await waitFor(() => expect(selectedNames()).toEqual(["3:Total"]));

    selectOnCanvas(null);
    await waitFor(() => expect(selectedNames(), "a click on no board clears the selection").toEqual([]));
  });
});
