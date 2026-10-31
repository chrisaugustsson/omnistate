// @vitest-environment jsdom
// US-058: big design docs load board by board. The design viewer (US-043)
// reads the head's outline first (US-112) and then the boards it shows, never
// the whole body at once (src/testing/design-boards.ts has the reads):
//
// - GET .../docs/<doc>/html?view=outline: the head's `outline`, its pages in
//   order, each with its boards under `children` as placeholders (each board's
//   own JSON, without its children), with the head's `components` and `tokens`.
// - GET .../docs/<doc>/boards/<board>: one board, its subtree as JSON in `node`.
//
// The renderer is shown the page with its boards as placeholders at first, so
// that zoomed out every board has its box on the canvas, and asked which
// boards its view shows: besides what web/src/design-viewer.test.tsx lists,
//
//   export type DesignRenderer = {
//     ...
//     // The IDs of the boards of the shown page that the current view shows
//     // (omni-render's `Scene::visible_boards`), in page order.
//     visibleBoards(): string[];
//   };
//
// The page asks after it shows a page and whenever the view changes (the
// listeners of `onView`), reads each board in view that it has not read yet,
// and shows the page again with the loaded boards in place of their
// placeholders; the renderer keeps its view when the same page is shown
// again. Boards out of view are not read. When the renderer cannot answer
// (an older build of its module, kept in the browser's cache, has no
// `_omni_render_visible_boards`), the page says so in its error line: no
// board could ever be read.
//
// A placeholder is drawn as a light neutral grey box (#e4e4e7) with the
// board's ID, name, place, size and rounding, not the board's own fills, so
// that a board not loaded yet is not taken for a dark design. (The API gives
// a board that hugs its content the size of that content in the outline.) It
// has the layout `{"mode": "none"}`: the renderer refuses a board without a
// layout, and a board without children needs no other.
//
// The app is rendered with its own router on a memory history, the browser's
// fetch is replaced by a fake of the API, and the browser's WebSocket by a fake
// socket.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { answerDesignRead, own, type JsonNode } from "./testing/design-boards";

/** What the fake renderers were asked to do, and which boards their view shows. */
const fake = vi.hoisted(() => {
  type Instance = {
    canvas: HTMLCanvasElement;
    shown: { page: unknown; system: unknown }[];
    viewListeners: Set<() => void>;
    destroyed: boolean;
  };
  return {
    instances: [] as Instance[],
    /** The boards every renderer's view shows; `null` for every board of the page shown. */
    visible: null as string[] | null,
    /** How often `visibleBoards` was asked. */
    asked: 0,
    /**
     * Whether the renderer's module is an older build without
     * `_omni_render_visible_boards`, as a browser that kept it in its cache
     * loads it: `visibleBoards` then throws what the real renderer throws.
     */
    stale: false,
  };
});

vi.mock("@/design/renderer", () => ({
  createDesignRenderer: async (canvas: HTMLCanvasElement) => {
    const instance = {
      canvas,
      shown: [] as { page: unknown; system: unknown }[],
      viewListeners: new Set<() => void>(),
      destroyed: false,
    };
    fake.instances.push(instance);
    const lastPage = () => instance.shown[instance.shown.length - 1]?.page as { children?: { id: string }[] } | undefined;
    return {
      show(page: unknown, system: unknown) {
        instance.shown.push({ page, system });
        // Showing a page draws a frame, which is a change of the view.
        for (const listener of Array.from(instance.viewListeners)) listener();
      },
      visibleBoards() {
        fake.asked += 1;
        if (fake.stale) throw new TypeError("module._omni_render_visible_boards is not a function");
        const boards = (lastPage()?.children ?? []).map((board) => String(board.id));
        return fake.visible === null ? boards : boards.filter((id) => fake.visible!.includes(id));
      },
      fontsStoodIn() {
        return [];
      },
      layers() {
        return [];
      },
      layersOf() {
        return null;
      },
      boardOf() {
        return null;
      },
      select() {},
      setMode() {},
      inspect() {
        return null;
      },
      boxOf() {
        return null;
      },
      onSelect() {
        return () => {};
      },
      onView(listener: () => void) {
        instance.viewListeners.add(listener);
        return () => instance.viewListeners.delete(listener);
      },
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
const DOC_ID = "0192d7a0-5858-7000-8000-000000000001";
const V1 = "0192d7a0-5858-7000-8000-000000000021";
const V2 = "0192d7a0-5858-7000-8000-000000000022";
const DS_V1 = "0192d7a0-5858-7000-8000-000000000023";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const AT = "2026-10-22T10:00:00Z";
const AUTHOR = { run: "run-1", role: "importer", model: "test-model", harness: "test-harness" };
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const HOME_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const DOC_PATH = `${HOME_PATH}/docs/${DOC_ID}`;
const DOC_TITLE = "TireBud Ordrar";

// The design ------------------------------------------------------------------------------
//
// Two pages. Ordrar holds three boards side by side, each with a text of its
// own; Arkiv holds one board. V2 changes the text of the first board.

const ORDRAR = "pg000001";
const ARKIV = "pg000002";
const ORDER_1 = "b0000001";
const ORDER_2 = "b0000002";
const ORDER_3 = "b0000003";
const ARCHIVE = "b0000004";
const ORDER_1_TEXT = "t0000001";
const fixed = (value: number) => ({ type: "fixed", value });

const board = (id: string, name: string, x: number, textId: string, content: string) => ({
  id,
  type: "board",
  name,
  x,
  y: 0,
  width: fixed(375),
  height: fixed(812),
  layout: { mode: "none" },
  fills: [{ type: "solid", color: "#ffffff" }],
  children: [
    { id: textId, type: "text", name: `${name} text`, content, fontFamily: "Inter", fontSize: 16, lineHeight: 1.5 },
  ],
});

function designPages(first: string): JsonNode[] {
  return [
    {
      id: ORDRAR,
      type: "page",
      name: "Ordrar",
      children: [
        board(ORDER_1, "Order 1", 0, ORDER_1_TEXT, first),
        board(ORDER_2, "Order 2", 400, "t0000002", "Order two content"),
        board(ORDER_3, "Order 3", 800, "t0000003", "Order three content"),
      ],
    },
    {
      id: ARKIV,
      type: "page",
      name: "Arkiv",
      children: [board(ARCHIVE, "Archive", 0, "t0000004", "Archive content")],
    },
  ];
}

const V1_FIRST = "Order one as it was";
const V2_FIRST = "Order one renamed";
const PAGES_OF: Record<string, JsonNode[]> = { [V1]: designPages(V1_FIRST), [V2]: designPages(V2_FIRST) };

const TOKENS = {
  $extensions: { omnistate: { modes: ["light"] } },
  color: { $type: "color", paper: { $value: "#ffffff" } },
};

// The API ---------------------------------------------------------------------------------

type Call = { method: string; path: string; params: URLSearchParams };

/**
 * A fake of the API: the auth routes, the project home, the design doc with
 * its versions V1 and V2, the board-by-board reads of the doc's head,
 * `headPages` (V1's pages until the test changes them). A whole version is
 * served too, for a page that still asks for it. With `holdBoards`, board
 * reads wait until `releaseBoards` is called.
 */
function fakeApi({ holdBoards = false }: { holdBoards?: boolean } = {}) {
  let release: () => void = () => {};
  const gate = holdBoards ? new Promise<void>((resolve) => (release = resolve)) : Promise.resolve();
  const api = { calls: [] as Call[], releaseBoards: () => release(), headPages: PAGES_OF[V1] };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
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
  const versionItem = (id: string) => ({
    id,
    doc_id: DOC_ID,
    draft_id: `${id.slice(0, -2)}99`,
    removed: false,
    created_at: AT,
    agent: AUTHOR,
  });
  const version = (id: string): JsonNode | null => {
    const pages = PAGES_OF[id];
    if (!pages) return null;
    return {
      ...versionItem(id),
      design_system_version_id: DS_V1,
      manifest: {
        pages: pages.map((page) => ({
          id: page.id,
          chunk: "0".repeat(64),
          boards: (page.children as JsonNode[]).map((entry) => ({ id: entry.id, chunk: "1".repeat(64) })),
        })),
      },
      nodes: [] as unknown[],
      tags: '<Page name="Ordrar"/>',
      pages,
      components: [] as unknown[],
      tokens: TOKENS,
    };
  };

  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(raw, "http://localhost:3000");
    const path = url.pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    api.calls.push({ method, path, params: url.searchParams });

    const read = answerDesignRead(API, method, url, (id) =>
      id === DOC_ID ? { ...version(V1), pages: api.headPages } : null,
    );
    if (read) {
      if (path.includes("/boards/")) await gate;
      return json(read.status, read.body);
    }
    switch (`${method} ${path}`) {
      case "GET /api/v1/auth/me":
        return json(200, { user: USER });
      case "GET /api/v1/auth/workspaces":
        return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
      case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
        return json(200, { projects: [{ id: PROJECT_ID, name: "TireBud", created_at: AT }] });
      case `GET ${API}/home`:
        return json(200, {
          project_id: PROJECT_ID,
          docs: [doc],
          threads: [],
          waiting: { comments: [] },
        });
      case `GET ${API}/docs/${DOC_ID}`:
        return json(200, { doc });
      case `GET ${API}/docs/${DOC_ID}/versions`:
        return json(200, { doc_id: DOC_ID, versions: [versionItem(V1), versionItem(V2)] });
      case `GET ${API}/docs/${DOC_ID}/html`:
        return json(200, { doc_id: DOC_ID, version_id: V1, html: "" });
      case `GET ${API}/comments`:
        return json(200, { project_id: PROJECT_ID, comments: [] });
      case `GET ${API}/versions/${V1}`:
        return json(200, { version: version(V1) });
      case `GET ${API}/versions/${V2}`:
        return json(200, { version: version(V2) });
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

// Reading what the page did ---------------------------------------------------------------

type Instance = (typeof fake.instances)[number];

/** The renderer that draws the canvas in `container`, once it has one. */
async function rendererIn(container: HTMLElement): Promise<Instance> {
  return waitFor(
    () => {
      const canvas = container.querySelector("canvas");
      expect(canvas, "a canvas").toBeTruthy();
      const instance = fake.instances.find((candidate) => candidate.canvas === canvas && !candidate.destroyed);
      expect(instance, "a renderer draws the canvas").toBeTruthy();
      return instance!;
    },
    { timeout: 3000 },
  );
}

async function viewerRenderer() {
  await screen.findByRole("heading", { name: DOC_TITLE }, { timeout: 3000 });
  return rendererIn(document.body);
}

function lastShown(instance: Instance): JsonNode | undefined {
  return instance.shown[instance.shown.length - 1]?.page as JsonNode | undefined;
}

function boardsIn(page: JsonNode | undefined): JsonNode[] {
  return Array.isArray(page?.children) ? (page.children as JsonNode[]) : [];
}

function boardIn(page: JsonNode | undefined, id: string): JsonNode | undefined {
  return boardsIn(page).find((entry) => entry.id === id);
}

/** The board `id` of `version` as the whole version holds it. */
function wholeBoard(version: string, id: string): JsonNode {
  for (const page of PAGES_OF[version]) {
    const found = boardsIn(page).find((entry) => entry.id === id);
    if (found) return found;
  }
  throw new Error(`no board ${id} in ${version}`);
}

/**
 * Whether `entry` is the placeholder of the board `id` of `version`: the
 * board's ID, name, place and size, none of its content, the layout every
 * board needs, and a light grey fill in place of its own look.
 */
function expectPlaceholder(entry: JsonNode | undefined, version: string, id: string) {
  expect(entry, `the board ${id} is on the page`).toBeTruthy();
  const children = Array.isArray(entry!.children) ? entry!.children : [];
  expect(children, `the board ${id} is a placeholder without its content`).toEqual([]);
  const whole = wholeBoard(version, id);
  expect(own(entry!), `the placeholder of ${id} has the board's name and box, filled light grey`).toEqual({
    id: whole.id,
    type: whole.type,
    name: whole.name,
    x: whole.x,
    y: whole.y,
    width: whole.width,
    height: whole.height,
    layout: { mode: "none" },
    fills: [{ type: "solid", color: "#e4e4e7" }],
  });
}

/** Waits until the renderer's last shown page is `page` with the board `id` of `version` loaded. */
async function loaded(instance: Instance, page: string, version: string, id: string) {
  await waitFor(
    () => {
      const shown = lastShown(instance);
      expect(shown?.id, `the renderer shows the page ${page}`).toBe(page);
      expect(boardIn(shown, id), `the board ${id} is loaded with its content`).toEqual(wholeBoard(version, id));
    },
    { timeout: 3000 },
  );
}

function boardReads(api: ReturnType<typeof fakeApi>, id?: string) {
  return api.calls.filter(
    (call) => call.method === "GET" && /\/docs\/[^/]+\/boards\/[^/]+$/.test(call.path) && (!id || call.path.endsWith(`/boards/${id}`)),
  );
}

function headReads(api: ReturnType<typeof fakeApi>) {
  return api.calls.filter((call) => call.method === "GET" && call.path === `${API}/docs/${DOC_ID}/html`);
}

function versionReads(api: ReturnType<typeof fakeApi>, version: string) {
  return api.calls.filter((call) => call.method === "GET" && call.path === `${API}/versions/${version}`);
}

/** Lets the page finish what it started. */
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
}

let api: ReturnType<typeof fakeApi>;

beforeEach(() => {
  fake.instances.length = 0;
  fake.visible = null;
  fake.asked = 0;
  fake.stale = false;
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

// The design viewer -----------------------------------------------------------------------

describe("the design viewer", () => {
  test("reads the head's outline first and never the whole body", async () => {
    api = fakeApi();
    fake.visible = [ORDER_1];
    renderAt(DOC_PATH);
    const renderer = await viewerRenderer();
    await loaded(renderer, ORDRAR, V1, ORDER_1);
    await settle();

    const reads = headReads(api);
    expect(reads.length, "the head is read").toBeGreaterThan(0);
    for (const read of reads) {
      expect(read.params.get("view"), `every read of the head asks for its outline: ?${read.params}`).toBe("outline");
    }
    expect(versionReads(api, V1), "no version is read").toEqual([]);
    expect(versionReads(api, V2), "no version is read").toEqual([]);
  });

  test("shows the page with every board as a placeholder until the boards are loaded", async () => {
    api = fakeApi({ holdBoards: true });
    fake.visible = [ORDER_1];
    renderAt(DOC_PATH);
    const renderer = await viewerRenderer();
    await waitFor(
      () => {
        const shown = lastShown(renderer);
        expect(shown?.id, "the page Ordrar is shown while its boards are still being read").toBe(ORDRAR);
        expect(boardsIn(shown).map((entry) => entry.id), "every board of the page, in order").toEqual([ORDER_1, ORDER_2, ORDER_3]);
      },
      { timeout: 3000 },
    );
    for (const id of [ORDER_1, ORDER_2, ORDER_3]) expectPlaceholder(boardIn(lastShown(renderer), id), V1, id);
    expect(fake.asked, "the page asks the renderer which boards are in view").toBeGreaterThan(0);

    api.releaseBoards();
    await loaded(renderer, ORDRAR, V1, ORDER_1);
    expectPlaceholder(boardIn(lastShown(renderer), ORDER_2), V1, ORDER_2);
    expectPlaceholder(boardIn(lastShown(renderer), ORDER_3), V1, ORDER_3);
  });

  test("a renderer that cannot say which boards are in view is reported in the error line, never kept quiet", async () => {
    api = fakeApi();
    fake.stale = true;
    renderAt(DOC_PATH);
    const renderer = await viewerRenderer();
    await waitFor(() => expect(lastShown(renderer)?.id, "the page Ordrar is shown with its placeholders").toBe(ORDRAR), {
      timeout: 3000,
    });
    const alert = await screen.findByRole("alert", {}, { timeout: 3000 });
    expect(alert.textContent).toMatch(/boards in view could not be found/i);
    expect(alert.textContent).toContain("_omni_render_visible_boards is not a function");
    await settle();
    expect(boardReads(api), "no board can be read").toEqual([]);
    expect(screen.getAllByRole("alert"), "said once, however often the view changes").toHaveLength(1);
  });

  test("reads only the boards in view, each from the head's board route", async () => {
    api = fakeApi();
    fake.visible = [ORDER_1];
    renderAt(DOC_PATH);
    const renderer = await viewerRenderer();
    await loaded(renderer, ORDRAR, V1, ORDER_1);
    await settle();

    const reads = boardReads(api);
    expect(reads.map((call) => call.path), "only the board in view is read").toEqual([`${API}/docs/${DOC_ID}/boards/${ORDER_1}`]);
    expect(reads[0].params.get("version"), "the board of the head").toBeNull();
    expect(boardReads(api, ORDER_2), "a board out of view is not read").toEqual([]);
    expect(boardReads(api, ORDER_3), "a board out of view is not read").toEqual([]);
  });

  test("boards that come into view as the reviewer pans load, and loaded boards are not read again", async () => {
    api = fakeApi();
    fake.visible = [ORDER_1];
    renderAt(DOC_PATH);
    const renderer = await viewerRenderer();
    await loaded(renderer, ORDRAR, V1, ORDER_1);

    // The reviewer pans to the right: Order 2 and Order 3 come into view.
    fake.visible = [ORDER_2, ORDER_3];
    act(() => {
      for (const listener of Array.from(renderer.viewListeners)) listener();
    });
    await loaded(renderer, ORDRAR, V1, ORDER_2);
    await loaded(renderer, ORDRAR, V1, ORDER_3);
    expect(boardIn(lastShown(renderer), ORDER_1), "the board loaded first stays loaded").toEqual(wholeBoard(V1, ORDER_1));
    await settle();
    expect(boardReads(api, ORDER_1), "a loaded board is read once").toHaveLength(1);
    expect(boardReads(api, ORDER_2)).toHaveLength(1);
    expect(boardReads(api, ORDER_3)).toHaveLength(1);
  });

  test("when an agent changes a board, the head is read again and only that board with it", async () => {
    api = fakeApi();
    renderAt(DOC_PATH);
    const renderer = await viewerRenderer();
    await loaded(renderer, ORDRAR, V1, ORDER_1);
    await loaded(renderer, ORDRAR, V1, ORDER_3);
    await settle();
    const before = headReads(api).length;

    await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
    const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
    socket.open();
    await waitFor(() =>
      expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })),
    );
    socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 40 });
    // An agent renames the first order: only its board changes.
    api.headPages = PAGES_OF[V2];
    socket.receive({ type: "event", seq: 41, event: "doc.changed", project_id: PROJECT_ID, at: AT, data: { doc_id: DOC_ID, agent: AUTHOR } });

    await loaded(renderer, ORDRAR, V2, ORDER_1);
    await settle();
    expect(headReads(api).length, "the head's outline is read again").toBeGreaterThan(before);
    expect(boardReads(api, ORDER_1), "the changed board is read again").toHaveLength(2);
    expect(boardReads(api, ORDER_2), "an unchanged board is not").toHaveLength(1);
    expect(boardReads(api, ORDER_3), "an unchanged board is not").toHaveLength(1);
    expect(fake.instances.filter((instance) => !instance.destroyed), "the same renderer, so the zoom and pan stay").toEqual([renderer]);
  });
});
