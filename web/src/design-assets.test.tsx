// @vitest-environment jsdom
// US-057: the design viewer (US-043) and the change set review's canvases
// (US-046) draw a design's own images and fonts. The app is rendered with its
// own router on a memory history, the browser's fetch is replaced by a fake of
// the API, and the browser's WebSocket by a fake socket.
//
// A design names its images as `attachment:<id>`: an image node's `image`, and
// the `image` of a fill of type "image" on a board, a frame or a vector, on the
// pages and in the components its instances use. Its design system attaches
// font files to its typography tokens: `$extensions.omnistate.fontFiles`, a
// list of `attachment:<id>`, for the family of the token's
// `$value.fontFamily` (docs/formats/tokens.md). Both are workspace attachments,
// served by GET /api/v1/w/<workspace id>/attachments/<id> as their bytes.
//
// The page reaches the renderer only through src/design/renderer.ts, which
// this test replaces with a fake. Besides what web/src/design-viewer.test.tsx
// lists, the renderer takes the bytes of images and fonts, which stay with it
// until it is destroyed (omni-render's `_omni_render_add_image` and
// `_omni_render_add_font`):
//
//   export type DesignRenderer = {
//     ...
//     // Supplies the image that `attachment` ("attachment:<id>") names, from
//     // its PNG or JPEG bytes. Throws when the renderer cannot read them.
//     addImage(attachment: string, bytes: Uint8Array): void;
//     // Supplies a font file for text in `family` ("Geist"). Throws when the
//     // renderer cannot read it.
//     addFont(family: string, bytes: Uint8Array): void;
//   };
//
// `show` lays a page out and draws it with the images and fonts supplied
// before it, so a page is shown (again) once its images and fonts are there.
// Every renderer is a module instance of its own, so each canvas (the
// review's base and new version each have one) is given what it draws.
//
// An image or a font that cannot be loaded (the API answers with an error, or
// the renderer refuses the bytes) is named in the viewer's error line, by its
// attachment ID or, for a font, by its family, and the rest of the design
// still draws: the design viewer's error line is its alert (role "alert"), a
// review canvas's is the text in its region.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { answerDesignRead } from "./testing/design-boards";

type Entry =
  | { kind: "image"; name: string; bytes: number[] }
  | { kind: "font"; name: string; bytes: number[] }
  | { kind: "show"; page: string };

/** What the fake renderers were asked to do, in order. */
const fake = vi.hoisted(() => {
  type FakeEntry =
    | { kind: "image"; name: string; bytes: number[] }
    | { kind: "font"; name: string; bytes: number[] }
    | { kind: "show"; page: string };
  type Instance = { canvas: HTMLCanvasElement; log: FakeEntry[]; destroyed: boolean };
  return {
    instances: [] as Instance[],
    /** Bytes that the renderer cannot read, as a font file or an image. */
    unreadable: [0xde, 0xad, 0xbe, 0xef],
  };
});

vi.mock("@/design/renderer", () => {
  const plain = (bytes: unknown): number[] => {
    if (bytes instanceof ArrayBuffer) return Array.from(new Uint8Array(bytes));
    if (ArrayBuffer.isView(bytes)) {
      return Array.from(new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength));
    }
    throw new Error(`the renderer takes bytes, not ${typeof bytes}`);
  };
  const unreadable = (bytes: number[]) =>
    bytes.length === fake.unreadable.length && bytes.every((byte, index) => byte === fake.unreadable[index]);
  return {
    createDesignRenderer: async (canvas: HTMLCanvasElement) => {
      const instance = { canvas, log: [] as Entry[], destroyed: false };
      fake.instances.push(instance);
      let lastPage: { children?: { id: string }[] } | undefined;
      return {
        addImage(attachment: string, bytes: Uint8Array) {
          const data = plain(bytes);
          if (unreadable(data)) throw new Error("the image is no PNG or JPEG");
          instance.log.push({ kind: "image", name: attachment, bytes: data });
        },
        addFont(family: string, bytes: Uint8Array) {
          const data = plain(bytes);
          if (unreadable(data)) throw new Error("the file is no font");
          instance.log.push({ kind: "font", name: family, bytes: data });
        },
        show(page: unknown) {
          lastPage = page as typeof lastPage;
          instance.log.push({ kind: "show", page: String((page as { id?: string }).id) });
        },
        // US-058's boards in view (src/design-boards.test.tsx): every board of the page shown.
        visibleBoards() {
          return (lastPage?.children ?? []).map((board) => String(board.id));
        },
        layers() {
          return [];
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
        onView() {
          return () => {};
        },
        destroy() {
          instance.destroyed = true;
        },
      };
    },
  };
});

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const DOC_ID = "0192d7a0-5757-7000-8000-000000000001";
const CS_MERGED = "0192d7a0-5757-7000-8000-000000000011";
const CS_ID = "0192d7a0-5757-7000-8000-000000000012";
const V1 = "0192d7a0-5757-7000-8000-000000000021";
const V2 = "0192d7a0-5757-7000-8000-000000000022";
const DS_V1 = "0192d7a0-5757-7000-8000-000000000023";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const AT = "2026-10-22T10:00:00Z";
const AUTHOR = { run: "run-1", role: "importer", model: "test-model", harness: "test-harness" };
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const ATTACHMENTS = `/api/v1/w/${WORKSPACE_ID}/attachments/`;
const HOME_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const DOC_PATH = `${HOME_PATH}/docs/${DOC_ID}`;
const REVIEW_PATH = `${HOME_PATH}/changesets/${CS_ID}`;
const DOC_TITLE = "TireBud Idag";
const CS_TITLE = "Import of tirebud-idag.penpot";

// The attachments -------------------------------------------------------------------------

const PHOTO = "0192d7a0-a771-7000-8000-000000000001";
const LOGO = "0192d7a0-a771-7000-8000-000000000002";
const ICON = "0192d7a0-a771-7000-8000-000000000003";
const HERO = "0192d7a0-a771-7000-8000-000000000004";
const MISSING = "0192d7a0-a771-7000-8000-000000000005";
const GEIST_REGULAR = "0192d7a0-a771-7000-8000-000000000011";
const GEIST_BOLD = "0192d7a0-a771-7000-8000-000000000012";
const GEIST_MONO = "0192d7a0-a771-7000-8000-000000000013";

const PNG_MAGIC = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const TTF_MAGIC = [0x00, 0x01, 0x00, 0x00];

/** The bytes the API serves for each attachment: a magic number and a byte of its own. */
const BYTES: Record<string, number[]> = {
  [PHOTO]: [...PNG_MAGIC, 1],
  [LOGO]: [...PNG_MAGIC, 2],
  [ICON]: [...PNG_MAGIC, 3],
  [HERO]: [...PNG_MAGIC, 4],
  [GEIST_REGULAR]: [...TTF_MAGIC, 11],
  [GEIST_BOLD]: [...TTF_MAGIC, 12],
  [GEIST_MONO]: [...TTF_MAGIC, 13],
};
const ref = (id: string) => `attachment:${id}`;

// The design ------------------------------------------------------------------------------
//
// Two pages. Idag holds the board Hem, filled with the image PHOTO, holding the
// image Logo (LOGO), the text Rubrik in Geist, the text Kod in Geist Mono and
// an instance of the component Badge, whose frame Ikon is filled with the image
// ICON. Kampanj holds the board Kampanj with the image Hero (HERO). The design
// system's typography tokens attach Geist Regular and Bold to Geist and Geist
// Mono Regular to Geist Mono.

const IDAG = "pg000001";
const KAMPANJ_PAGE = "pg000002";
const HEM = "b0000001";
const KAMPANJ = "b0000002";
const fixed = (value: number) => ({ type: "fixed", value });
const text = (id: string, name: string, content: string, fontFamily: string) => ({
  id,
  type: "text",
  name,
  content,
  fontFamily,
  fontSize: 16,
  lineHeight: 1.5,
});

function designPages({ broken }: { broken: boolean }) {
  return [
    {
      id: IDAG,
      type: "page",
      name: "Idag",
      children: [
        {
          id: HEM,
          type: "board",
          name: "Hem",
          x: 0,
          y: 0,
          width: fixed(375),
          height: fixed(812),
          layout: { mode: "none" },
          fills: [{ type: "image", image: ref(PHOTO), fit: "cover" }],
          children: [
            { id: "i0000001", type: "image", name: "Logo", image: ref(LOGO), width: fixed(64), height: fixed(64) },
            ...(broken
              ? [{ id: "i0000002", type: "image", name: "Trasig", image: ref(MISSING), width: fixed(64), height: fixed(64) }]
              : []),
            text("t0000001", "Rubrik", "Dina däck", "Geist"),
            text("t0000002", "Kod", "ABC 123", "Geist Mono"),
            { id: "n0000001", type: "instance", name: "Märke", component: "cb000001", x: 20, y: 300 },
          ],
        },
      ],
    },
    {
      id: KAMPANJ_PAGE,
      type: "page",
      name: "Kampanj",
      children: [
        {
          id: KAMPANJ,
          type: "board",
          name: "Kampanj",
          x: 0,
          y: 0,
          width: fixed(375),
          height: fixed(812),
          layout: { mode: "none" },
          children: [
            { id: "i0000003", type: "image", name: "Hero", image: ref(HERO), width: fixed(375), height: fixed(200) },
          ],
        },
      ],
    },
  ];
}

const COMPONENTS = [
  {
    id: "cb000001",
    type: "component",
    name: "Badge",
    width: fixed(80),
    height: fixed(24),
    layout: { mode: "none" },
    children: [
      {
        id: "cf000001",
        type: "frame",
        name: "Ikon",
        width: fixed(16),
        height: fixed(16),
        layout: { mode: "none" },
        fills: [{ type: "image", image: ref(ICON), fit: "contain" }],
      },
    ],
  },
];

const typography = (fontFamily: string, files: string[]) => ({
  $value: { fontFamily, fontSize: "16px", fontWeight: 400, lineHeight: 1.5, letterSpacing: "0px" },
  $extensions: { omnistate: { fontFiles: files.map(ref) } },
});

const TOKENS = {
  $extensions: { omnistate: { modes: ["Mörkt", "Ljust"] } },
  typography: {
    $type: "typography",
    body: typography("Geist", [GEIST_REGULAR]),
    heading: { large: typography("Geist", [GEIST_BOLD]) },
    code: typography("Geist Mono", [GEIST_MONO]),
  },
};

// The API ---------------------------------------------------------------------------------

type Call = { method: string; path: string };

/**
 * A fake of the API: the auth routes, the project home, the design doc with
 * its versions V1 (mainline) and V2 (under review in CS_ID), the change set's
 * review and the attachments. The attachments in `failing` answer 500 and those
 * in `unreadable` serve bytes the renderer cannot read. With `broken`, the
 * board Hem also holds an image of the attachment MISSING, which the API does
 * not have.
 */
function fakeApi({
  broken = false,
  failing = [] as string[],
  unreadable = [] as string[],
}: { broken?: boolean; failing?: string[]; unreadable?: string[] } = {}) {
  const api = { calls: [] as Call[] };
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
  const versionItem = (id: string, changesetId: string) => ({
    id,
    doc_id: DOC_ID,
    changeset_id: changesetId,
    draft_id: `${id.slice(0, -2)}99`,
    removed: false,
    created_at: AT,
    agent: AUTHOR,
  });
  const changeset = (id: string, title: string, status: string) => ({
    id,
    project_id: PROJECT_ID,
    title,
    description: "",
    status,
    run_tag: "run-1",
    created_at: AT,
    history: [] as unknown[],
    approved_versions: [] as unknown[],
    decisions: [] as unknown[],
  });
  const changesets = [changeset(CS_MERGED, "Start the design", "merged"), changeset(CS_ID, CS_TITLE, "in_review")];
  const version = (id: string) => {
    const pages = designPages({ broken });
    return {
      ...versionItem(id, id === V1 ? CS_MERGED : CS_ID),
      design_system_version_id: DS_V1,
      manifest: {
        pages: pages.map((page) => ({
          id: page.id,
          chunk: "0".repeat(64),
          boards: page.children.map((board) => ({ id: board.id, chunk: "1".repeat(64) })),
        })),
      },
      nodes: [] as unknown[],
      tags: '<Page name="Idag"/>',
      pages,
      components: COMPONENTS,
      tokens: TOKENS,
    };
  };
  const node = (id: string, type: string, parent: string | null) => ({ id, type, parent, fingerprint: `fp-${id}` });

  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const url = new URL(raw, "http://localhost:3000");
    const path = url.pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    api.calls.push({ method, path });

    // US-058: a version's outline and its boards, one by one.
    const read = answerDesignRead(
      API,
      method,
      url,
      (id) => (id === V1 || id === V2 ? version(id) : null),
      (id) => (id === DOC_ID ? version(V1) : null),
    );
    if (read) return json(read.status, read.body);
    if (method === "GET" && path.startsWith(ATTACHMENTS)) {
      const id = path.slice(ATTACHMENTS.length);
      if (failing.includes(id)) return json(500, { error: "internal" });
      const bytes = unreadable.includes(id) ? fake.unreadable : BYTES[id];
      if (!bytes) return json(404, { error: "not_found" });
      const type = bytes[0] === 0x89 ? "image/png" : "font/ttf";
      return new Response(new Uint8Array(bytes), { status: 200, headers: { "content-type": type } });
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
          changesets,
          threads: [],
          waiting: { decisions: [], changesets: [], comments: [], questions: [] },
        });
      case `GET ${API}/changesets`:
        return json(200, { changesets });
      case `GET ${API}/changesets/${CS_ID}`:
        return json(200, { changeset: changesets[1] });
      case `GET ${API}/changesets/${CS_ID}/review`:
        return json(200, {
          changeset: changesets[1],
          docs: [
            {
              doc_id: DOC_ID,
              kind: "design",
              pillar: "ui",
              title: DOC_TITLE,
              base_version_id: V1,
              version_id: V2,
              removed: false,
              changes: {
                added: [] as unknown[],
                removed: [] as unknown[],
                changed: [node("t0000001", "text", HEM)],
                moved: [] as unknown[],
              },
            },
          ],
          decisions: [] as unknown[],
        });
      case `GET ${API}/changesets/${CS_ID}/conflicts`:
        return json(200, { conflicts: [] });
      case `GET ${API}/docs/${DOC_ID}`:
        return json(200, { doc });
      case `GET ${API}/docs/${DOC_ID}/versions`:
        return json(200, { doc_id: DOC_ID, versions: [versionItem(V1, CS_MERGED), versionItem(V2, CS_ID)] });
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

/** A WebSocket that stays connecting: these pages need no live events. */
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

// Reading what the renderers were given -----------------------------------------------------

type Instance = (typeof fake.instances)[number];

/** The renderer that draws the canvas in `container`, once it has one. */
async function rendererIn(container: HTMLElement): Promise<Instance> {
  return waitFor(() => {
    const canvas = container.querySelector("canvas");
    expect(canvas, "a canvas").toBeTruthy();
    const instance = fake.instances.find((candidate) => candidate.canvas === canvas && !candidate.destroyed);
    expect(instance, "a renderer draws the canvas").toBeTruthy();
    return instance!;
  });
}

/**
 * Waits until the renderer was given `images` and `fonts` (each family with
 * the bytes of its files) and then showed `page`: the page drawn with them.
 */
async function drawnWith(
  instance: Instance,
  page: string,
  images: string[],
  fonts: [family: string, attachment: string][],
) {
  await waitFor(() => {
    const log = instance.log;
    const shows = log.flatMap((entry, index) => (entry.kind === "show" && entry.page === page ? [index] : []));
    expect(shows.length, `the renderer showed the page ${page}`).toBeGreaterThan(0);
    const lastShow = shows[shows.length - 1];
    for (const id of images) {
      const at = log.findIndex(
        (entry) => entry.kind === "image" && entry.name === ref(id) && sameBytes(entry.bytes, BYTES[id]),
      );
      expect(at, `the image ${ref(id)} was given to the renderer with its bytes: ${logOf(log)}`).toBeGreaterThanOrEqual(0);
      expect(at, `the page ${page} was shown after the image ${ref(id)} was given`).toBeLessThan(lastShow);
    }
    for (const [family, id] of fonts) {
      const at = log.findIndex(
        (entry) => entry.kind === "font" && entry.name === family && sameBytes(entry.bytes, BYTES[id]),
      );
      expect(at, `the font file ${ref(id)} was given to the renderer for ${family}: ${logOf(log)}`).toBeGreaterThanOrEqual(0);
      expect(at, `the page ${page} was shown after the font file ${ref(id)} was given`).toBeLessThan(lastShow);
    }
  });
}

function sameBytes(a: number[], b: number[]) {
  return a.length === b.length && a.every((byte, index) => byte === b[index]);
}

function logOf(log: Entry[]) {
  return JSON.stringify(log.map((entry) => (entry.kind === "show" ? `show ${entry.page}` : `${entry.kind} ${entry.name}`)));
}

/** The error line of the design viewer: its alerts' text. */
function alertText() {
  return screen
    .queryAllByRole("alert")
    .map((alert) => alert.textContent ?? "")
    .join(" ");
}

const ALL_FONTS: [string, string][] = [
  ["Geist", GEIST_REGULAR],
  ["Geist", GEIST_BOLD],
  ["Geist Mono", GEIST_MONO],
];

async function findCanvasArea() {
  await screen.findByRole("heading", { name: DOC_TITLE }, { timeout: 3000 });
  return waitFor(() => {
    const canvas = document.querySelector("canvas");
    expect(canvas, "the design viewer has a canvas").toBeTruthy();
    return document.body;
  });
}

async function findReviewRegions() {
  const docs = await screen.findByRole("region", { name: /^(docs|changes|changed docs)$/i }, { timeout: 3000 });
  const entry = await waitFor(() => {
    const named = within(docs).getAllByText(DOC_TITLE)[0];
    const found = named.closest("li, article, [role=listitem]");
    expect(found, "the design's entry").toBeTruthy();
    return found as HTMLElement;
  });
  const base = await within(entry).findByRole("region", { name: /^base\b/i }, { timeout: 3000 });
  const after = await within(entry).findByRole("region", { name: /^new version\b/i }, { timeout: 3000 });
  return { base, after };
}

let api: ReturnType<typeof fakeApi>;

beforeEach(() => {
  fake.instances.length = 0;
  vi.stubGlobal("WebSocket", FakeWebSocket);
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

// Images ----------------------------------------------------------------------------------

describe("images", () => {
  test("the design viewer gives the renderer every image of the page shown and of its components, then draws it", async () => {
    api = fakeApi();
    renderAt(DOC_PATH);
    const renderer = await rendererIn(await findCanvasArea());
    await drawnWith(renderer, IDAG, [PHOTO, LOGO, ICON], []);
    for (const id of [PHOTO, LOGO, ICON]) {
      expect(
        api.calls.some((call) => call.method === "GET" && call.path === `${ATTACHMENTS}${id}`),
        `the image ${id} is read from the workspace's attachments`,
      ).toBe(true);
    }
    expect(alertText(), "nothing failed").toBe("");
  });

  test("both canvases of the change set review are given the images of the version they draw", async () => {
    api = fakeApi();
    renderAt(REVIEW_PATH);
    const { base, after } = await findReviewRegions();
    const baseRenderer = await rendererIn(base);
    const newRenderer = await rendererIn(after);
    expect(baseRenderer, "each side has its own renderer").not.toBe(newRenderer);
    await drawnWith(baseRenderer, IDAG, [PHOTO, LOGO, ICON], []);
    await drawnWith(newRenderer, IDAG, [PHOTO, LOGO, ICON], []);
  });
});

// Fonts -----------------------------------------------------------------------------------

describe("fonts", () => {
  test("the design viewer gives the renderer the font files of the design system's typography tokens, by family", async () => {
    api = fakeApi();
    renderAt(DOC_PATH);
    const renderer = await rendererIn(await findCanvasArea());
    await drawnWith(renderer, IDAG, [], ALL_FONTS);
    for (const id of [GEIST_REGULAR, GEIST_BOLD, GEIST_MONO]) {
      expect(
        api.calls.some((call) => call.method === "GET" && call.path === `${ATTACHMENTS}${id}`),
        `the font file ${id} is read from the workspace's attachments`,
      ).toBe(true);
    }
  });
});

// Failures --------------------------------------------------------------------------------

describe("an image or a font that fails to load", () => {
  test("an image the API does not have is named in the design viewer's error line, and the rest still draws", async () => {
    api = fakeApi({ broken: true });
    renderAt(DOC_PATH);
    const renderer = await rendererIn(await findCanvasArea());
    await drawnWith(renderer, IDAG, [PHOTO, LOGO, ICON], ALL_FONTS);
    await waitFor(() => expect(alertText(), "the error line names the image").toContain(MISSING));
    // It stays once the page is drawn.
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(alertText(), "the error line still names the image").toContain(MISSING);
    expect(renderer.log.some((entry) => entry.kind === "image" && entry.name === ref(MISSING))).toBe(false);
  });

  test("a font file the API fails to serve is named in the design viewer's error line, and the rest still draws", async () => {
    api = fakeApi({ failing: [GEIST_MONO] });
    renderAt(DOC_PATH);
    const renderer = await rendererIn(await findCanvasArea());
    await drawnWith(renderer, IDAG, [PHOTO, LOGO, ICON], [ALL_FONTS[0], ALL_FONTS[1]]);
    await waitFor(() => {
      const said = alertText();
      expect(said.includes("Geist Mono") || said.includes(GEIST_MONO), `the error line names the font: ${said}`).toBe(true);
    });
  });

  test("a font file the renderer cannot read is named in the design viewer's error line, and the rest still draws", async () => {
    api = fakeApi({ unreadable: [GEIST_BOLD] });
    renderAt(DOC_PATH);
    const renderer = await rendererIn(await findCanvasArea());
    await drawnWith(renderer, IDAG, [PHOTO, LOGO, ICON], [ALL_FONTS[0], ALL_FONTS[2]]);
    await waitFor(() => {
      const said = alertText();
      expect(said.includes("Geist") || said.includes(GEIST_BOLD), `the error line names the font: ${said}`).toBe(true);
    });
  });

  test("an image that fails on a review canvas is named in that canvas's region, and the rest still draws", async () => {
    api = fakeApi({ broken: true });
    renderAt(REVIEW_PATH);
    const { after } = await findReviewRegions();
    const renderer = await rendererIn(after);
    await drawnWith(renderer, IDAG, [PHOTO, LOGO, ICON], ALL_FONTS);
    await waitFor(() => expect(after.textContent ?? "", "the canvas's error line names the image").toContain(MISSING));
  });
});
