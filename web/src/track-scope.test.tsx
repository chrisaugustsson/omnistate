// @vitest-environment jsdom
// Where an item lives. The doc and design viewers show the doc's breadcrumb
// (US-112): scope › pillar › title, the scope ("App" at the app level, else the
// track's name) linking to its page and the pillar to its section there. The
// app is rendered with its own router on a memory history, the browser's fetch
// is replaced by a fake of the API, and the browser's WebSocket by a fake socket
// that never opens, so a page shows what it loads itself.
//
// The fake serves the project's home, the tracks and every doc (GET .../docs
// and GET .../docs/<doc>, each DocBody with its `track_id`).
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

// The design viewer draws on a canvas with the WASM renderer; these tests look
// at what is around it only.
vi.mock("@/design/renderer", () => ({
  createDesignRenderer: async () => ({
    show() {},
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
    onSelect() {
      return () => {};
    },
    setMode() {},
    inspect() {
      return null;
    },
    boxOf() {
      return null;
    },
    onView() {
      return () => {};
    },
    visibleBoards() {
      return [];
    },
    addImage() {},
    addFont() {},
    destroy() {},
  }),
}));

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
/** The part every track of the fixtures is on (US-305). */
const PART_ID = "0192d7a0-3333-7000-8000-000000000001";
const PROJECT_NAME = "Storefront";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-23T10:00:00Z";

const WISHLIST = "0192d7a0-7777-7000-8000-000000000001";
const LOYALTY = "0192d7a0-7777-7000-8000-000000000003";

let counter = 0;
/** A UUIDv7-shaped ID that no other fixture has. */
function newId() {
  counter += 1;
  return `0192d7a0-9999-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

function trackPath(trackId: string, section = "") {
  return `${PROJECT_PATH}/tracks/${trackId}${section ? `/${section}` : ""}`;
}

/**
 * A track as the API sends it. `legacyStatus` is the `status` field that
 * features had before US-110: the fixtures keep it so that they parse against
 * the API schema before and after the field goes, and so that the tests also
 * show that the app pays no heed to it.
 */
function track(id: string, name: string, description: string, legacyStatus: string) {
  return { id, project_id: PROJECT_ID, part_id: PART_ID, state: "planning", name, description, status: legacyStatus, created_at: AT };
}

function doc(trackId: string | null, kind: string, pillar: string, title: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    track_id: trackId,
    kind,
    pillar,
    title,
    created_at: AT,
    mainline_version_id: null,
    drafts: [],
    claims: [],
  };
}

const HTML = '<section data-omni-id="s0000001"><p data-omni-id="p0000001">Saved items.</p></section>';

/**
 * A project with two global docs (an HTML doc and the design system), a
 * track (Wishlist) with an HTML doc and a design, and another track
 * (Loyalty points, whose legacy status says done). No doc's title holds the
 * project's or a track's name.
 */
function seeded() {
  const wishlist = track(WISHLIST, "Wishlist", "Shoppers save products for later.", "active");
  const loyalty = track(LOYALTY, "Loyalty points", "Earn points on every order.", "done");

  const shopPrd = doc(null, "html", "spec", "Shop PRD");
  const designSystem = doc(null, "design-system", "ui", "Design system");
  const savedPrd = doc(WISHLIST, "html", "spec", "Saved items PRD");
  const savedScreens = doc(WISHLIST, "design", "ui", "Saved items screens");
  const pointsPrd = doc(LOYALTY, "html", "spec", "Points PRD");

  const home = {
    project_id: PROJECT_ID,
    docs: [shopPrd, designSystem, savedPrd, savedScreens, pointsPrd],
    threads: [],
    waiting: { comments: [] },
    tracks: [
      { track: wishlist, docs: 2, waiting: 0, stories: { total: 3, open: 2, done: 1 } },
      { track: loyalty, docs: 1, waiting: 0, stories: { total: 0, open: 0, done: 0 } },
    ],
  };
  return {
    home,
    tracks: [wishlist, loyalty],
    docs: { savedPrd, savedScreens, shopPrd },
  };
}

type Seeded = ReturnType<typeof seeded>;
type Call = { method: string; path: string };

/** A fake of the API; it records every call that is not a read. */
function fakeApi(seed: Seeded) {
  const { home } = seed;
  const calls: Call[] = [];
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url, "http://localhost:3000").pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    if (method !== "GET") calls.push({ method, path });
    switch (`${method} ${path}`) {
      case "GET /api/v1/auth/me":
        return json(200, { user: USER });
      case "GET /api/v1/auth/workspaces":
        return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
      case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
        return json(200, { projects: [{ id: PROJECT_ID, name: PROJECT_NAME, created_at: AT }] });
      case `GET ${API}/home`:
        return json(200, home);
      case `GET ${API}/tracks`:
        return json(200, { tracks: seed.tracks });
      case `GET ${API}/docs`:
        return json(200, { docs: home.docs });
      case `GET ${API}/comments`:
        return json(200, { project_id: PROJECT_ID, comments: [] });
      default:
        break;
    }
    for (const f of seed.tracks) {
      if (method === "GET" && path === `${API}/tracks/${f.id}`) return json(200, { track: f });
    }
    if (method !== "GET") return json(404, { error: "not_found" });
    for (const d of home.docs) {
      if (path === `${API}/docs/${d.id}`) return json(200, { doc: d });
      if (path === `${API}/docs/${d.id}/versions`) return json(200, { doc_id: d.id, versions: [] });
      if (path === `${API}/docs/${d.id}/html`) return json(200, { doc_id: d.id, html: HTML, version_id: null });
    }
    return json(404, { error: "not_found" });
  });
  vi.stubGlobal("fetch", fetch);
  return { calls };
}

/** A WebSocket that never opens: these pages are read once. */
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

  send() {
    throw new Error("InvalidStateError: the socket is not open");
  }

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

/** Asserts that a doc's breadcrumb is exactly `crumbs`, the scope and the pillar linking to `hrefs`. */
async function expectDocTrail(crumbs: string[], hrefs: string[]) {
  const trail = await screen.findByRole("navigation", { name: /breadcrumb/i });
  await waitFor(() =>
    expect(within(trail).getAllByRole("listitem").map((item) => (item.textContent ?? "").trim())).toEqual(crumbs),
  );
  expect(Array.from(trail.querySelectorAll("a")).map((a) => [a.textContent, a.getAttribute("href")])).toEqual([
    [crumbs[0], hrefs[0]],
    [crumbs[1], hrefs[1]],
  ]);
}

describe("track scope", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  test("a track's header has a chip with its done of total stories, as the Tracks list counts them, that opens its board", async () => {
    fakeApi(seeded());
    renderAt(trackPath(WISHLIST));
    const chips = await screen.findByRole("navigation", { name: /^pillars$/i });
    const chip = await within(chips).findByRole("link", { name: "Stories 1/3 done" });
    expect(chip.getAttribute("href")).toBe(trackPath(WISHLIST, "board"));
  });

  test("a track's header has a Behaviour link beside its stories chip that opens its behaviour screen", async () => {
    fakeApi(seeded());
    renderAt(trackPath(WISHLIST));
    const chips = await screen.findByRole("navigation", { name: /^pillars$/i });
    const link = await within(chips).findByRole("link", { name: "Behaviour" });
    expect(link.getAttribute("href")).toBe(trackPath(WISHLIST, "behaviour"));
  });

  describe("where an item lives", () => {
    test("a track's doc in the doc viewer: track › pillar › title, linking the track's page and the pillar on it", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/docs/${seed.docs.savedPrd.id}`);
      await screen.findByRole("heading", { level: 1, name: "Saved items PRD" });
      await expectDocTrail(["Wishlist", "Spec", "Saved items PRD"], [trackPath(WISHLIST), `${trackPath(WISHLIST)}#spec`]);
    });

    test("a track's design in the design viewer: track › UI › title", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/docs/${seed.docs.savedScreens.id}`);
      await screen.findByRole("heading", { level: 1, name: "Saved items screens" });
      await expectDocTrail(["Wishlist", "UI", "Saved items screens"], [trackPath(WISHLIST), `${trackPath(WISHLIST)}#ui`]);
    });

    test("an app-level doc: App › pillar › title, linking the App page", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/docs/${seed.docs.shopPrd.id}`);
      await screen.findByRole("heading", { level: 1, name: "Shop PRD" });
      await expectDocTrail(["App", "Spec", "Shop PRD"], [`${PROJECT_PATH}/app`, `${PROJECT_PATH}/app#spec`]);
    });
  });
});
