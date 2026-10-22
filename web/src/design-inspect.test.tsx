// @vitest-environment jsdom
// US-044: the design viewer's inspect panel and mode switch. The reviewer
// selects a node (in the layers panel or on the canvas, US-043) and sees every
// property of it, with each token's name next to the value it has in the
// current mode; for an instance, the component, the property values and the
// overrides. A mode switch (light and dark here) redraws the design with that
// mode's token values. The app is rendered with its own router on a memory
// history at the design doc's page, the browser's fetch is replaced by a fake
// of the API, and the browser's WebSocket by a fake socket.
//
// The page reaches the renderer (omni-render on WebGL, which jsdom cannot
// run) only through src/design/renderer.ts, which this test replaces with a
// fake. Besides what US-043 uses (show, layers, select, onSelect, destroy,
// src/design-viewer.test.tsx), the renderer gives:
//
//   // Resolves the shown page's tokens in `mode` from now on and draws it
//   // again (omni-render's `_omni_render_scene_set_mode`). `show` resolves a
//   // page in the first mode of its design system, so the page calls
//   // `setMode` again after showing another page in another mode.
//   setMode(mode: string): void;
//   // The node at `address` in the current mode, as omni-render's
//   // `_omni_render_inspect` gives it (crates/omni-render/tests/inspect.rs),
//   // or null when no node has that address.
//   inspect(address: string): Inspection | null;
//
//   type Inspection = {
//     address: string; name: string; type: string; mode: string | null;
//     box: { x: number; y: number; width: number; height: number };
//     layout: Property[]; size: Property[]; style: Property[]; text: Property[];
//     instance: null | {
//       component: string; componentName: string;
//       props: { name: string; kind: string; value: unknown; set: boolean }[];
//       overrides: { node: string; name: string; fields: Record<string, unknown> }[];
//     };
//   };
//   type Property = { name: string; value: unknown; tokens: { token: string; value: unknown }[] };
//
// The page shows:
//
// - an inspect panel: a region named "Inspect". With nothing selected it has
//   no tables. With a node selected it shows the node's name and one table
//   per group of properties that the node has, named (aria-label or caption)
//   "Layout", "Size", "Style" and "Text", with a row per property. A row's
//   first cell is the property's name, and the row shows its value (numbers
//   and text as they are) and, for each token the property uses, the token's
//   name and its value in the current mode. The Size table's "width" and
//   "height" rows also show the node's laid-out size from `box`. For an
//   instance it also has a table named "Component", whose row "component"
//   shows the component's name and which has a row per property value
//   (marked "default" where the instance sets none), and a table named
//   "Overrides" with a row per override, whose first cell is the overridden
//   layer's name, showing the fields and their values.
// - a mode switch: a select named "Mode" with an option per mode of the
//   version's token tree (`$extensions.omnistate.modes`), in order, the first
//   chosen at first. Choosing a mode redraws the design in it, keeps the
//   selection, and the inspect panel shows that mode's values. The mode stays
//   when another page is shown.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { answerDesignRead } from "./testing/design-boards";

type Layer = { address: string; name: string; type: string; children: Layer[] };
type TokenUse = { token: string; value: unknown };
type Property = { name: string; value: unknown; tokens: TokenUse[] };
type Inspection = {
  address: string;
  name: string;
  type: string;
  mode: string | null;
  box: { x: number; y: number; width: number; height: number };
  layout: Property[];
  size: Property[];
  style: Property[];
  text: Property[];
  instance: null | {
    component: string;
    componentName: string;
    props: { name: string; kind: string; value: unknown; set: boolean }[];
    overrides: { node: string; name: string; fields: Record<string, unknown> }[];
  };
};

/** What the fake renderers were asked to do. */
const fake = vi.hoisted(() => {
  type FakeLayer = { address: string; name: string; type: string; children: FakeLayer[] };
  type Instance = {
    canvas: HTMLCanvasElement;
    shown: { page: unknown; system: unknown }[];
    selected: (string | null)[];
    /** The modes `setMode` was called with, in order. */
    modeCalls: string[];
    /** The mode the shown page is drawn in. */
    mode: string | null;
    listeners: Set<(address: string | null) => void>;
    destroyed: boolean;
  };
  return {
    instances: [] as Instance[],
    /** The layers the renderer gives for each page, by the page's ID. */
    layersByPage: new Map<string, FakeLayer[]>(),
    /** What the renderer gives for a node, by `<mode> <address>`. */
    inspections: new Map<string, unknown>(),
  };
});

vi.mock("@/design/renderer", () => ({
  createDesignRenderer: async (canvas: HTMLCanvasElement) => {
    const instance = {
      canvas,
      shown: [] as { page: unknown; system: unknown }[],
      selected: [] as (string | null)[],
      modeCalls: [] as string[],
      mode: null as string | null,
      listeners: new Set<(address: string | null) => void>(),
      destroyed: false,
    };
    fake.instances.push(instance);
    return {
      show(page: unknown, system: unknown) {
        instance.shown.push({ page, system });
        const modes = (system as { $extensions?: { omnistate?: { modes?: string[] } } } | null)?.$extensions
          ?.omnistate?.modes;
        instance.mode = modes?.[0] ?? null;
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
      setMode(mode: string) {
        instance.modeCalls.push(mode);
        instance.mode = mode;
      },
      inspect(address: string) {
        return fake.inspections.get(`${instance.mode} ${address}`) ?? null;
      },
      // US-045's comment pins (src/design-review.test.tsx); this test does
      // not look at them.
      boxOf() {
        return null;
      },
      onView() {
        return () => {};
      },
      // US-058's boards in view (src/design-boards.test.tsx): every board of the page shown.
      visibleBoards() {
        const last = instance.shown[instance.shown.length - 1];
        const boards = (last?.page as { children?: { id: string }[] } | undefined)?.children ?? [];
        return boards.map((board) => String(board.id));
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
// Two pages. Mobile holds the board Cart: Buy (an instance of Button with
// label "Pay now", overriding Label's fill with $color.accent) and Summary (a
// frame padded $spacing.pad, filled $color.surface, holding the text Total).
// Desktop holds the board Wide cart (the text Heading). Button (filled
// $color.brand, properties label and showIcon) has the layers Label and Icon.

const MOBILE = "pg000001";
const DESKTOP = "pg000002";
const CART_BOARD = "a0000001";
const WIDE_CART = "a0000003";
const BUY = "e0000001";
const SUMMARY = "f0000001";
const TOTAL = "f0000002";
const HEADING = "f0000003";
const CART = "cc000001";
const BUTTON = "cb000001";
const LABEL = "ct000001";
const ICON = "cn000001";

const fixed = (value: number) => ({ type: "fixed", value });
const solid = (color: string) => [{ type: "solid", color }];
const pad = (value: number | string) => ({ top: value, right: value, bottom: value, left: value });

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
        fills: solid("$color.surface"),
        children: [
          {
            id: BUY,
            type: "instance",
            name: "Buy",
            component: BUTTON,
            x: 20,
            y: 20,
            props: { label: "Pay now" },
            overrides: { [LABEL]: { fills: solid("$color.accent") } },
          },
          {
            id: SUMMARY,
            type: "frame",
            name: "Summary",
            x: 20,
            y: 100,
            width: "hug",
            height: "hug",
            fills: solid("$color.surface"),
            layout: { mode: "flex", padding: pad("$spacing.pad") },
            children: [
              {
                id: TOTAL,
                type: "text",
                name: "Total",
                width: "hug",
                height: "hug",
                content: "Total 96 kr",
                fontFamily: "Inter",
                fontSize: 14,
                lineHeight: 1.25,
                color: "#000000",
              },
            ],
          },
        ],
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
            color: "$color.ink",
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
  },
  {
    id: BUTTON,
    type: "component",
    name: "Button",
    width: fixed(120),
    height: fixed(40),
    layout: { mode: "none" },
    fills: solid("$color.brand"),
    props: {
      label: { type: "text", default: "Buy" },
      showIcon: { type: "boolean", default: true },
    },
    children: [
      {
        id: LABEL,
        type: "text",
        name: "Label",
        x: 40,
        y: 10,
        content: "Buy",
        fontFamily: "Inter",
        fontSize: 16,
        lineHeight: 1.25,
        color: "$color.ink",
        bind: { content: "label" },
      },
      { id: ICON, type: "instance", name: "Icon", component: CART, x: 10, y: 10, bind: { visible: "showIcon" } },
    ],
  },
];

/** The token values of each mode. */
const PALETTE = {
  light: { surface: "#ffffff", brand: "#ff0000", ink: "#000000", accent: "#00a000", pad: 12 },
  dark: { surface: "#000000", brand: "#0000ff", ink: "#ffffff", accent: "#80ff80", pad: 28 },
};

const perMode = (light: string, dark: string) => ({
  $value: light,
  $extensions: { omnistate: { modes: { light, dark } } },
});

const TOKENS = {
  $extensions: { omnistate: { modes: ["light", "dark"] } },
  color: {
    $type: "color",
    surface: perMode(PALETTE.light.surface, PALETTE.dark.surface),
    brand: perMode(PALETTE.light.brand, PALETTE.dark.brand),
    ink: perMode(PALETTE.light.ink, PALETTE.dark.ink),
    accent: perMode(PALETTE.light.accent, PALETTE.dark.accent),
  },
  spacing: { $type: "dimension", pad: perMode("12px", "28px") },
};

function layer(address: string, name: string, type: string, children: Layer[] = []): Layer {
  return { address, name, type, children };
}

function seedLayers() {
  fake.layersByPage.set(MOBILE, [
    layer(CART_BOARD, "Cart", "board", [
      layer(BUY, "Buy", "instance", [
        layer(`${BUY}/${LABEL}`, "Label", "text"),
        layer(`${BUY}/${ICON}`, "Icon", "instance"),
      ]),
      layer(SUMMARY, "Summary", "frame", [layer(TOTAL, "Total", "text")]),
    ]),
  ]);
  fake.layersByPage.set(DESKTOP, [layer(WIDE_CART, "Wide cart", "board", [layer(HEADING, "Heading", "text")])]);
}

const property = (name: string, value: unknown, tokens: TokenUse[] = []): Property => ({ name, value, tokens });

/** What the renderer gives for the nodes of the design in `mode`. */
function seedInspections() {
  for (const mode of ["light", "dark"] as const) {
    const p = PALETTE[mode];
    const add = (inspection: Omit<Inspection, "mode">) =>
      fake.inspections.set(`${mode} ${inspection.address}`, { ...inspection, mode });
    add({
      address: SUMMARY,
      name: "Summary",
      type: "frame",
      box: mode === "light" ? { x: 20, y: 100, width: 224, height: 60 } : { x: 20, y: 100, width: 256, height: 92 },
      layout: [
        property("layout", { mode: "flex", padding: pad(p.pad) }, [{ token: "$spacing.pad", value: p.pad }]),
        property("x", 20),
        property("y", 100),
      ],
      size: [property("width", "hug"), property("height", "hug")],
      style: [property("fills", solid(p.surface), [{ token: "$color.surface", value: p.surface }])],
      text: [],
      instance: null,
    });
    add({
      address: TOTAL,
      name: "Total",
      type: "text",
      box: { x: 20 + p.pad, y: 100 + p.pad, width: 200, height: 36 },
      layout: [],
      size: [property("width", "hug"), property("height", "hug")],
      style: [],
      text: [
        property("content", "Total 96 kr"),
        property("fontFamily", "Inter"),
        property("fontSize", 14),
        property("lineHeight", 1.25),
        property("color", "#000000"),
      ],
      instance: null,
    });
    add({
      address: BUY,
      name: "Buy",
      type: "instance",
      box: { x: 20, y: 20, width: 120, height: 40 },
      layout: [property("layout", { mode: "none" }), property("x", 20), property("y", 20)],
      size: [property("width", fixed(120)), property("height", fixed(40))],
      style: [property("fills", solid(p.brand), [{ token: "$color.brand", value: p.brand }])],
      text: [],
      instance: {
        component: BUTTON,
        componentName: "Button",
        props: [
          { name: "label", kind: "text", value: "Pay now", set: true },
          { name: "showIcon", kind: "boolean", value: true, set: false },
        ],
        overrides: [{ node: LABEL, name: "Label", fields: { fills: solid("$color.accent") } }],
      },
    });
    add({
      address: `${BUY}/${LABEL}`,
      name: "Label",
      type: "text",
      box: { x: 60, y: 30, width: 61, height: 20 },
      layout: [property("x", 40), property("y", 10)],
      size: [],
      style: [],
      text: [
        property("content", "Pay now"),
        property("fontFamily", "Inter"),
        property("fontSize", 16),
        property("lineHeight", 1.25),
        property("color", p.accent, [{ token: "$color.accent", value: p.accent }]),
      ],
      instance: null,
    });
    add({
      address: HEADING,
      name: "Heading",
      type: "text",
      box: { x: 0, y: 0, width: 150, height: 40 },
      layout: [],
      size: [],
      style: [],
      text: [
        property("content", "Your cart"),
        property("fontSize", 32),
        property("color", p.ink, [{ token: "$color.ink", value: p.ink }]),
      ],
      instance: null,
    });
  }
}

/** A fake of the API: the auth routes, the project, the design doc and its mainline version. */
function fakeApi() {
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
  const version = {
    ...item,
    design_system_version_id: DS_V1,
    manifest,
    nodes: [],
    tags: '<Page name="Mobile"/>',
    pages: PAGES,
    components: COMPONENTS,
    tokens: TOKENS,
  };

  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(raw, "http://localhost:3000");
    const path = url.pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();

    // US-058: the version's outline and its boards, one by one.
    const read = answerDesignRead(API, method, url, (id) => (id === V1 ? version : null));
    if (read) return json(read.status, read.body);
    switch (`${method} ${path}`) {
      case "GET /api/v1/auth/me":
        return json(200, { user: USER });
      case "GET /api/v1/auth/workspaces":
        return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
      case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
        return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
      case `GET ${API}/changesets`:
        return json(200, { changesets });
      case `GET ${API}/docs/${DOC_ID}`:
        return json(200, { doc });
      case `GET ${API}/docs/${DOC_ID}/versions`:
        return json(200, { doc_id: DOC_ID, versions: [item] });
      case `GET ${API}/comments`:
        return json(200, { project_id: PROJECT_ID, comments: [] });
      case `GET ${API}/versions/${V1}`:
        return json(200, { version });
      default:
        return json(404, { error: "not_found" });
    }
  });
  vi.stubGlobal("fetch", fetch);
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
    expect(document.body.contains(latest.canvas), "the renderer's canvas is on the page").toBe(true);
    return latest;
  });
}

function lastShownPage(renderer: Awaited<ReturnType<typeof findRenderer>>) {
  return (renderer.shown[renderer.shown.length - 1]?.page as { id?: string } | undefined)?.id;
}

/** The treeitem of the node named `name`, once the layers panel lists it. */
async function findTreeItem(name: string) {
  return waitFor(() => {
    const region = screen.getByRole("region", { name: /^layers$/i });
    return within(region).getByRole("treeitem", { name: new RegExp(`^${name}\\b`) });
  });
}

function inspectPanel() {
  return screen.getByRole("region", { name: /^inspect$/i });
}

function queryTable(name: string) {
  return within(inspectPanel()).queryByRole("table", { name: new RegExp(`^${name}$`, "i") });
}

/** The row of the table `table` in the inspect panel whose first cell is `first`. */
function row(table: string, first: string) {
  const element = queryTable(table);
  expect(element, `the inspect panel has a table named ${table}`).toBeTruthy();
  const rows = within(element!).queryAllByRole("row");
  const found = rows.find((candidate) => (candidate.querySelector("th, td")?.textContent ?? "").trim() === first);
  expect(
    found,
    `the ${table} table has a row for ${first}: ${rows.map((r) => (r.textContent ?? "").trim()).join(" | ")}`,
  ).toBeTruthy();
  return (found!.textContent ?? "").replace(/\s+/g, " ");
}

/** Waits until the row `first` of `table` shows every text of `shown`. */
async function expectRow(table: string, first: string, shown: string[], hidden: string[] = []) {
  await waitFor(() => {
    const text = row(table, first);
    for (const part of shown) expect(text, `the ${table} row ${first} shows ${part}`).toContain(part);
    for (const part of hidden) expect(text, `the ${table} row ${first} does not show ${part}`).not.toContain(part);
  });
}

function modeSwitch() {
  return screen.getByRole("combobox", { name: /^mode$/i }) as HTMLSelectElement;
}

async function findModeSwitch() {
  return waitFor(() => {
    const select = modeSwitch();
    expect(select.options.length, "the mode switch lists the modes").toBeGreaterThan(0);
    return select;
  });
}

/** The reviewer selects `address` on the canvas. */
function selectOnCanvas(address: string | null) {
  const renderer = fake.instances[fake.instances.length - 1];
  act(() => {
    for (const listener of Array.from(renderer.listeners)) listener(address);
  });
}

beforeEach(() => {
  fake.instances.length = 0;
  fake.layersByPage.clear();
  fake.inspections.clear();
  seedLayers();
  seedInspections();
  fakeApi();
  vi.stubGlobal("WebSocket", FakeWebSocket);
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

// The inspect panel ----------------------------------------------------------------------

describe("the inspect panel", () => {
  test("shows the selected node's layout, size and style, with each token next to its value", async () => {
    renderAt(DOC_PATH);
    const summary = await findTreeItem("Summary");
    expect(inspectPanel(), "the inspect panel is there").toBeTruthy();
    expect(
      within(inspectPanel()).queryAllByRole("table"),
      "with nothing selected there is nothing to inspect",
    ).toEqual([]);

    fireEvent.click(summary);
    await waitFor(() => expect(within(inspectPanel()).queryAllByText(/Summary/).length, "the node's name").toBeGreaterThan(0));
    await expectRow("Layout", "x", ["20"]);
    await expectRow("Layout", "y", ["100"]);
    await expectRow("Layout", "layout", ["flex", "$spacing.pad", "12"]);
    await expectRow("Size", "width", ["hug", "224"]);
    await expectRow("Size", "height", ["hug", "60"]);
    await expectRow("Style", "fills", ["$color.surface", "#ffffff"]);
    expect(queryTable("Component"), "a frame is no instance").toBeNull();
    expect(queryTable("Overrides")).toBeNull();
  });

  test("shows a text's text properties, also for a node selected on the canvas", async () => {
    renderAt(DOC_PATH);
    await findTreeItem("Total");
    await waitFor(() => expect(fake.instances[fake.instances.length - 1].listeners.size).toBeGreaterThan(0));

    selectOnCanvas(TOTAL);
    await expectRow("Text", "content", ["Total 96 kr"]);
    await expectRow("Text", "fontFamily", ["Inter"]);
    await expectRow("Text", "fontSize", ["14"]);
    await expectRow("Text", "lineHeight", ["1.25"]);
    await expectRow("Text", "color", ["#000000"]);

    selectOnCanvas(`${BUY}/${LABEL}`);
    await expectRow("Text", "content", ["Pay now"], ["Total 96 kr"]);
    await expectRow("Text", "color", ["$color.accent", "#00a000"]);

    selectOnCanvas(null);
    await waitFor(() =>
      expect(within(inspectPanel()).queryAllByRole("table"), "a cleared selection shows nothing").toEqual([]),
    );
  });
});

describe("an instance in the inspect panel", () => {
  test("shows its component, its property values and its overrides", async () => {
    renderAt(DOC_PATH);
    fireEvent.click(await findTreeItem("Buy"));

    await expectRow("Component", "component", ["Button"]);
    await expectRow("Component", "label", ["Pay now"], ["default"]);
    await expectRow("Component", "showIcon", ["true", "default"]);
    await expectRow("Overrides", "Label", ["fills", "$color.accent"]);
    await expectRow("Style", "fills", ["$color.brand", "#ff0000"]);
    await expectRow("Size", "width", ["120"]);
    await expectRow("Size", "height", ["40"]);
  });
});

// The mode switch ------------------------------------------------------------------------

describe("the mode switch", () => {
  test("lists the design system's modes with the first one chosen", async () => {
    renderAt(DOC_PATH);
    const select = await findModeSwitch();
    expect(Array.from(select.options).map((option) => option.textContent?.trim())).toEqual(["light", "dark"]);
    expect(select.value).toBe("light");
  });

  test("redraws the design in the chosen mode, and the inspect panel shows that mode's values", async () => {
    renderAt(DOC_PATH);
    const renderer = await findRenderer();
    fireEvent.click(await findTreeItem("Summary"));
    await expectRow("Style", "fills", ["#ffffff"]);
    const select = await findModeSwitch();

    fireEvent.change(select, { target: { value: "dark" } });
    await waitFor(() => expect(renderer.mode, "the renderer draws the design in dark mode").toBe("dark"));
    expect(renderer.modeCalls[renderer.modeCalls.length - 1]).toBe("dark");
    expect(fake.instances[fake.instances.length - 1], "the same renderer redraws it").toBe(renderer);
    expect(modeSwitch().value).toBe("dark");

    await expectRow("Style", "fills", ["$color.surface", "#000000"], ["#ffffff"]);
    await expectRow("Layout", "layout", ["$spacing.pad", "28"]);
    await expectRow("Size", "width", ["256"]);
    const item = await findTreeItem("Summary");
    expect(item.getAttribute("aria-selected"), "the selection stays").toBe("true");

    fireEvent.change(modeSwitch(), { target: { value: "light" } });
    await waitFor(() => expect(renderer.mode).toBe("light"));
    await expectRow("Style", "fills", ["#ffffff"], ["#000000"]);
  });

  test("keeps the chosen mode when another page is shown", async () => {
    renderAt(DOC_PATH);
    const renderer = await findRenderer();
    const select = await findModeSwitch();
    await waitFor(() => expect(lastShownPage(renderer)).toBe(MOBILE));
    fireEvent.change(select, { target: { value: "dark" } });
    await waitFor(() => expect(renderer.mode).toBe("dark"));

    const pages = screen.queryByRole("navigation", { name: /^pages$/i }) ?? screen.getByRole("region", { name: /^pages$/i });
    fireEvent.click(within(pages).getByRole("button", { name: /^Desktop\b/ }));
    await waitFor(() => expect(lastShownPage(renderer)).toBe(DESKTOP));
    await waitFor(() => expect(renderer.mode, "Desktop is drawn in dark mode too").toBe("dark"));
    expect(modeSwitch().value).toBe("dark");

    fireEvent.click(await findTreeItem("Heading"));
    await expectRow("Text", "color", ["$color.ink", "#ffffff"]);
  });
});
