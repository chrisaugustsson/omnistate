// @vitest-environment jsdom
// US-046: the change set review, where the reviewer sees exactly what a change
// set changes and approves or rejects it in one place. It is
// /w/<workspace id>/projects/<project id>/changesets/<change set id>, and the
// project home (US-040) links each change set's title in its "Change sets"
// section there. The app is rendered with its own router on a memory history,
// the browser's fetch is replaced by a fake of the API, and the browser's
// WebSocket by a fake socket.
//
// The page loads (paths under /api/v1/w/<workspace>/projects/<project>):
//
// - GET /changesets/<id>/review (crates/omni-api/tests/changeset_review.rs):
//   `{changeset, docs, decisions}`. `docs` lists each doc the change set
//   changes, oldest first, as `{doc_id, kind, module, title, base_version_id,
//   version_id, removed, changes: {added, removed, changed, moved}}`, the
//   node changes from the base version to the version under review.
//   `decisions` are the linked decisions, each the full Decision.
// - GET /changesets/<id>/conflicts (US-026): the conflicts its rebases
//   recorded, each with its `node`, `kind`, both sides and its `resolution`
//   (`{note, resolved_at}`, or `null` while unresolved).
// - For a design: GET /versions/<base> and GET /versions/<version> (their
//   `pages`, `components` and `tokens`). For an HTML doc: GET
//   /docs/<doc>/html?version=<version> (and of the base, if the page shows it).
//
// It shows:
//
// - the change set's title as a heading, and its status;
// - a section named "Docs" (a heading names it; "Changes" or "Changed docs"
//   also do) with one entry per doc, a list item or an article (`li`,
//   `article` or role `listitem`) holding the doc's title and a summary of its
//   node changes: how many nodes were added, changed and removed, as
//   "<n> added", "<n> changed" and "<n> removed". Every entry shows its
//   changes without a click:
//   - a design: two regions inside the entry, named "Base" and "New version",
//     each with a canvas drawn by its own renderer (src/design/renderer.ts,
//     faked here as in web/src/design-review.test.tsx). Both show the same
//     page, at first the first page of the new version that holds a change;
//     the base region shows it as the base version has it, the new version
//     region as the version under review has it. Over each canvas a highlight
//     marks every changed node that the renderer can place (`boxOf`): the
//     added and changed nodes over the new version, the removed nodes over the
//     base. A highlight is an element with `data-change` ("added", "changed"
//     or "removed") and `data-node` (the node's address), placed with inline
//     `left`, `top`, `width` and `height` in px from the canvas's top-left
//     corner on the node's box, and it follows the view (zoom and pan). Nodes
//     that did not change have no highlight.
//   - an HTML doc: the viewer frame of US-042 (`<iframe sandbox=
//     "allow-scripts">` with the viewer's document), to which the page posts
//     `viewer.show` with the HTML of the version under review and `changes`:
//     `[{node, change}]` for the blocks the review lists as added or changed
//     (web/src/doc-viewer-frame.test.ts tests that the frame marks them).
//     Blocks that did not change are not in `changes`.
// - a section named "Decisions" listing each linked decision by its direction,
//   with its status ("proposed", "approved", "rejected", "superseded").
// - an "Approve" button (or "Approve change set") and a "Reject" button,
//   while the change set is in review. While any linked decision is not
//   approved, Approve is disabled and its accessible description
//   (`aria-describedby`, or its `title`) names the first such decision by its
//   direction. Approve sends POST /changesets/<id>/approve `{}`. Reject first
//   shows a field named "Reason" and a confirming "Reject" button (the last
//   button named "Reject…" on the page) that sends nothing while the reason is
//   blank, then POST /changesets/<id>/reject `{reason}`. Afterwards the page
//   shows the change set's new status ("merged", "rejected").
// - a section named "Conflicts" listing each conflict of the change set's
//   rebases with its node ID and how it was resolved: the resolution's note,
//   or that it is unresolved ("unresolved" or "not resolved").
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { answerDesignRead } from "./testing/design-boards";

type Box = { x: number; y: number; width: number; height: number };
type JsonNode = { id?: string; name?: string; type?: string; content?: string; children?: JsonNode[] };

/** What the fake renderers were asked to do. */
const fake = vi.hoisted(() => {
  type FakeBox = { x: number; y: number; width: number; height: number };
  type Instance = {
    canvas: HTMLCanvasElement;
    shown: { page: unknown; system: unknown }[];
    viewListeners: Set<() => void>;
    view: { x: number; y: number; zoom: number };
    destroyed: boolean;
  };
  return {
    instances: [] as Instance[],
    /** The page box of each node, by its address. */
    boxes: new Map<string, FakeBox>(),
  };
});

vi.mock("@/design/renderer", () => {
  type Node = { id?: string; children?: Node[] };
  const has = (nodes: Node[], id: string): boolean =>
    nodes.some((node) => node.id === id || has(node.children ?? [], id));
  return {
    createDesignRenderer: async (canvas: HTMLCanvasElement) => {
      const instance = {
        canvas,
        shown: [] as { page: unknown; system: unknown }[],
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
        layers() {
          return [];
        },
        select() {},
        setMode() {},
        inspect() {
          return null;
        },
        // A node has a box only when the page shown holds it.
        boxOf(address: string) {
          const page = lastPage();
          if (!page || !has([page], address)) return null;
          const box = fake.boxes.get(address);
          if (!box) return null;
          const { x, y, zoom } = instance.view;
          return { x: (box.x - x) * zoom, y: (box.y - y) * zoom, width: box.width * zoom, height: box.height * zoom };
        },
        onSelect() {
          return () => {};
        },
        onView(listener: () => void) {
          instance.viewListeners.add(listener);
          return () => instance.viewListeners.delete(listener);
        },
        // US-058's boards in view (src/design-boards.test.tsx): every board of the page shown.
        visibleBoards() {
          return ((lastPage()?.children ?? []) as { id?: string }[]).map((board) => String(board.id));
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
const CS_ID = "0192d7a0-8888-7000-8000-000000000011";
const CS_MERGED = "0192d7a0-8888-7000-8000-000000000012";
const DESIGN_DOC = "0192d7a0-8888-7000-8000-000000000001";
const HTML_DOC = "0192d7a0-8888-7000-8000-000000000002";
const DESIGN_BASE = "0192d7a0-8888-7000-8000-000000000021";
const DESIGN_NEW = "0192d7a0-8888-7000-8000-000000000022";
const HTML_BASE = "0192d7a0-8888-7000-8000-000000000023";
const HTML_NEW = "0192d7a0-8888-7000-8000-000000000024";
const THREAD_A = "0192d7a0-8888-7000-8000-000000000031";
const THREAD_B = "0192d7a0-8888-7000-8000-000000000032";
const DECISION_APPROVED = "0192d7a0-8888-7000-8000-000000000041";
const DECISION_PROPOSED = "0192d7a0-8888-7000-8000-000000000042";
const CONFLICT_RESOLVED = "0192d7a0-8888-7000-8000-000000000051";
const CONFLICT_OPEN = "0192d7a0-8888-7000-8000-000000000052";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const AT = "2026-10-21T10:00:00Z";
const AUTHOR = { run: "run-1", role: "author", model: "test-model", harness: "test-harness" };
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const HOME_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const REVIEW_PATH = `${HOME_PATH}/changesets/${CS_ID}`;
const CS_TITLE = "Faster shipping";
const DESIGN_TITLE = "Checkout screens";
const HTML_TITLE = "Checkout PRD";
const APPROVED_DIRECTION = "Ship every order within one day";
const PROPOSED_DIRECTION = "Drop the gift wrap option";
const RESOLVED_NOTE = "Kept one day, as the decision says";

// The design --------------------------------------------------------------------------
//
// Two pages. On Mobile, the board Cart holds the instance Buy and the frame
// Summary with the text Total, and the board Receipt is empty in the base. The
// new version removes Buy, changes Total and adds the text Note to Receipt; on
// Desktop it changes the text Heading of the board Wide cart.

const MOBILE = "pg000001";
const DESKTOP = "pg000002";
const CART_BOARD = "a0000001";
const RECEIPT = "a0000002";
const WIDE_CART = "a0000003";
const BUY = "e0000001";
const SUMMARY = "f0000001";
const TOTAL = "f0000002";
const HEADING = "f0000003";
const NOTE = "f0000004";
const BUTTON = "cb000001";

const fixed = (value: number) => ({ type: "fixed", value });
const text = (id: string, name: string, content: string) => ({
  id,
  type: "text",
  name,
  content,
  fontFamily: "Inter",
  fontSize: 14,
  lineHeight: 1.25,
});

function designPages(base: boolean) {
  const cart: JsonNode[] = [
    {
      id: SUMMARY,
      type: "frame",
      name: "Summary",
      children: [text(TOTAL, "Total", base ? "Total 96 kr" : "Total 120 kr")],
    },
  ];
  if (base) cart.unshift({ id: BUY, type: "instance", name: "Buy" });
  return [
    {
      id: MOBILE,
      type: "page",
      name: "Mobile",
      children: [
        { id: CART_BOARD, type: "board", name: "Cart", x: 0, y: 0, width: fixed(375), height: fixed(812), children: cart },
        {
          id: RECEIPT,
          type: "board",
          name: "Receipt",
          x: 400,
          y: 0,
          width: fixed(375),
          height: fixed(812),
          children: base ? [] : [text(NOTE, "Note", "Thanks for your order")],
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
          children: [text(HEADING, "Heading", base ? "Your cart" : "Your basket")],
        },
      ],
    },
  ];
}

const COMPONENTS = [
  { id: BUTTON, type: "component", name: "Button", width: fixed(120), height: fixed(40), children: [] },
];

function seedBoxes() {
  fake.boxes.set(CART_BOARD, { x: 0, y: 0, width: 375, height: 812 });
  fake.boxes.set(BUY, { x: 20, y: 20, width: 120, height: 40 });
  fake.boxes.set(SUMMARY, { x: 20, y: 100, width: 200, height: 100 });
  fake.boxes.set(TOTAL, { x: 20, y: 100, width: 80, height: 18 });
  fake.boxes.set(RECEIPT, { x: 400, y: 0, width: 375, height: 812 });
  fake.boxes.set(NOTE, { x: 420, y: 40, width: 100, height: 20 });
  fake.boxes.set(WIDE_CART, { x: 0, y: 0, width: 1280, height: 800 });
  fake.boxes.set(HEADING, { x: 0, y: 0, width: 150, height: 40 });
}

// The HTML doc --------------------------------------------------------------------------
//
// The new version rewrites p0000001, drops p0000003 and adds p0000004.

const HTML_BASE_HTML = `<section data-node-id="s0000001">
<h1 data-node-id="h0000001">Checkout</h1>
<p data-node-id="p0000001">Orders ship in two days.</p>
<p data-node-id="p0000002">Refunds take a week.</p>
<p data-node-id="p0000003">Gift wrap is extra.</p>
</section>`;
const HTML_NEW_HTML = `<section data-node-id="s0000001">
<h1 data-node-id="h0000001">Checkout</h1>
<p data-node-id="p0000001">Orders ship in one day.</p>
<p data-node-id="p0000002">Refunds take a week.</p>
<p data-node-id="p0000004">Returns are free.</p>
</section>`;

// The API -------------------------------------------------------------------------------

function node(id: string, type: string, parent: string | null) {
  return { id, type, parent, fingerprint: `fp-${id}` };
}

function reviewedDocs() {
  return [
    {
      doc_id: DESIGN_DOC,
      kind: "design",
      module: "ui",
      title: DESIGN_TITLE,
      base_version_id: DESIGN_BASE,
      version_id: DESIGN_NEW,
      removed: false,
      changes: {
        added: [node(NOTE, "text", RECEIPT)],
        removed: [node(BUY, "instance", CART_BOARD)],
        changed: [node(TOTAL, "text", SUMMARY), node(HEADING, "text", WIDE_CART)],
        moved: [] as unknown[],
      },
    },
    {
      doc_id: HTML_DOC,
      kind: "html",
      module: "prd",
      title: HTML_TITLE,
      base_version_id: HTML_BASE,
      version_id: HTML_NEW,
      removed: false,
      changes: {
        added: [node("p0000004", "paragraph", "s0000001")],
        removed: [node("p0000003", "paragraph", "s0000001")],
        changed: [node("p0000001", "paragraph", "s0000001")],
        moved: [] as unknown[],
      },
    },
  ];
}

function decision(id: string, threadId: string, direction: string, status: string) {
  return {
    id,
    project_id: PROJECT_ID,
    thread_id: threadId,
    changeset_id: CS_ID,
    status,
    author: { kind: "agent", ...AUTHOR },
    direction,
    chosen: "One day",
    criteria: ["Cheap to run"],
    options: [
      { name: "One day", scores: { "Cheap to run": 6 }, lost_because: null as string | null },
      { name: "Two days", scores: { "Cheap to run": 8 }, lost_because: "Customers leave for faster shops" },
    ],
    assumptions: [] as string[],
    open_questions: [] as string[],
    dissent: [] as unknown[],
    posts: [] as string[],
    rejection_reason: null as string | null,
    created_at: AT,
  };
}

function conflicts() {
  return [
    {
      id: CONFLICT_RESOLVED,
      changeset_id: CS_ID,
      doc_id: HTML_DOC,
      node: "p0000001",
      kind: "both_changed",
      mainline: { version_id: HTML_BASE, html: '<p data-node-id="p0000001">Orders ship in three days.</p>' },
      changeset: { version_id: HTML_NEW, html: '<p data-node-id="p0000001">Orders ship in one day.</p>' },
      resolution: { note: RESOLVED_NOTE, resolved_at: AT },
    },
    {
      id: CONFLICT_OPEN,
      changeset_id: CS_ID,
      doc_id: HTML_DOC,
      node: "p0000004",
      kind: "unplaced",
      mainline: { version_id: HTML_BASE, html: null as string | null },
      changeset: { version_id: HTML_NEW, html: '<p data-node-id="p0000004">Returns are free.</p>' },
      resolution: null as { note: string; resolved_at: string } | null,
    },
  ];
}

type Call = { method: string; path: string; body: Record<string, unknown> | null };

/**
 * A fake of the API: the auth routes, the project home, the change set's
 * review and conflicts, and the versions. With `undecided`, one linked
 * decision is still proposed.
 */
function fakeApi({ undecided }: { undecided: boolean }) {
  const api = {
    status: "in_review",
    rejectionReason: null as string | null,
    calls: [] as Call[],
  };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  const decisions = () => [
    decision(DECISION_APPROVED, THREAD_A, APPROVED_DIRECTION, "approved"),
    decision(DECISION_PROPOSED, THREAD_B, PROPOSED_DIRECTION, undecided ? "proposed" : "approved"),
  ];
  const changeset = () => ({
    id: CS_ID,
    project_id: PROJECT_ID,
    title: CS_TITLE,
    description: "Orders ship in one day, and the receipt thanks the buyer.",
    status: api.status,
    run_tag: "run-1",
    created_at: AT,
    history: [{ from_status: null as string | null, to_status: "open", actor: "agent", reason: null as string | null, at: AT }],
    approved_versions: [] as unknown[],
    decisions: decisions().map((d) => ({ id: d.id, thread_id: d.thread_id, status: d.status })),
  });
  const merged = {
    id: CS_MERGED,
    project_id: PROJECT_ID,
    title: "Write the checkout PRD",
    description: "",
    status: "merged",
    run_tag: "run-1",
    created_at: AT,
    history: [] as unknown[],
    approved_versions: [] as unknown[],
    decisions: [] as unknown[],
  };
  const versionItem = (id: string, docId: string, changesetId: string) => ({
    id,
    doc_id: docId,
    changeset_id: changesetId,
    draft_id: `${id.slice(0, -2)}99`,
    removed: false,
    created_at: AT,
    agent: AUTHOR,
  });
  const designVersion = (id: string) => {
    const base = id === DESIGN_BASE;
    const pages = designPages(base);
    return {
      ...versionItem(id, DESIGN_DOC, base ? CS_MERGED : CS_ID),
      design_system_version_id: null as string | null,
      manifest: {
        pages: pages.map((page) => ({
          id: page.id,
          chunk: "0".repeat(64),
          boards: page.children.map((board) => ({ id: board.id, chunk: "1".repeat(64) })),
        })),
      },
      nodes: [] as unknown[],
      tags: '<Page name="Mobile"/>',
      pages,
      components: COMPONENTS,
      tokens: null as unknown,
    };
  };
  const htmlOf: Record<string, string> = { [HTML_BASE]: HTML_BASE_HTML, [HTML_NEW]: HTML_NEW_HTML };
  const docBody = (id: string, kind: string, module: string, title: string, mainline: string) => ({
    id,
    project_id: PROJECT_ID,
    kind,
    module,
    title,
    created_at: AT,
    mainline_version_id: mainline,
    drafts: [] as unknown[],
    claims: [] as unknown[],
  });
  const docs = () => [
    docBody(DESIGN_DOC, "design", "ui", DESIGN_TITLE, DESIGN_BASE),
    docBody(HTML_DOC, "html", "prd", HTML_TITLE, HTML_BASE),
  ];

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

    // US-058: a version's outline and its boards, one by one.
    const read = answerDesignRead(API, method, url, (id) =>
      id === DESIGN_BASE || id === DESIGN_NEW ? designVersion(id) : null,
    );
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
          docs: docs(),
          changesets: [merged, changeset()],
          threads: [],
          waiting: { decisions: [], changesets: [], comments: [], questions: [] },
        });
      case `GET ${API}/changesets`:
        return json(200, { changesets: [merged, changeset()] });
      case `GET ${API}/changesets/${CS_ID}`:
        return json(200, { changeset: changeset() });
      case `GET ${API}/changesets/${CS_ID}/review`:
        return json(200, { changeset: changeset(), docs: reviewedDocs(), decisions: decisions() });
      case `GET ${API}/changesets/${CS_ID}/conflicts`:
        return json(200, { conflicts: conflicts() });
      case `POST ${API}/changesets/${CS_ID}/approve`: {
        if (api.status !== "in_review") return json(409, { error: "invalid_transition" });
        if (undecided) return json(409, { error: "decision_not_approved", decision_id: DECISION_PROPOSED });
        api.status = "merged";
        return json(200, { changeset: changeset() });
      }
      case `POST ${API}/changesets/${CS_ID}/reject`: {
        const reason = typeof body?.reason === "string" ? body.reason.trim() : "";
        if (!reason) return json(400, { error: "invalid_request" });
        if (api.status !== "in_review") return json(409, { error: "invalid_transition" });
        api.status = "rejected";
        api.rejectionReason = reason;
        return json(200, { changeset: changeset() });
      }
      case `GET ${API}/docs/${DESIGN_DOC}`:
        return json(200, { doc: docs()[0] });
      case `GET ${API}/docs/${HTML_DOC}`:
        return json(200, { doc: docs()[1] });
      case `GET ${API}/docs/${DESIGN_DOC}/versions`:
        return json(200, {
          doc_id: DESIGN_DOC,
          versions: [versionItem(DESIGN_BASE, DESIGN_DOC, CS_MERGED), versionItem(DESIGN_NEW, DESIGN_DOC, CS_ID)],
        });
      case `GET ${API}/docs/${HTML_DOC}/versions`:
        return json(200, {
          doc_id: HTML_DOC,
          versions: [versionItem(HTML_BASE, HTML_DOC, CS_MERGED), versionItem(HTML_NEW, HTML_DOC, CS_ID)],
        });
      case `GET ${API}/docs/${HTML_DOC}/html`: {
        const id = url.searchParams.get("version") ?? HTML_BASE;
        if (!htmlOf[id]) return json(404, { error: "not_found" });
        return json(200, { doc_id: HTML_DOC, version_id: id, html: htmlOf[id] });
      }
      case `GET ${API}/docs/${DESIGN_DOC}/html`:
        // A design version has no HTML.
        return json(200, { doc_id: DESIGN_DOC, version_id: url.searchParams.get("version") ?? DESIGN_BASE, html: "" });
      case `GET ${API}/comments`:
        return json(200, { project_id: PROJECT_ID, comments: [] });
      default:
        break;
    }
    if (method === "GET" && (path === `${API}/versions/${DESIGN_BASE}` || path === `${API}/versions/${DESIGN_NEW}`)) {
      return json(200, { version: designVersion(path.slice(path.lastIndexOf("/") + 1)) });
    }
    if (method === "GET" && (path === `${API}/versions/${HTML_BASE}` || path === `${API}/versions/${HTML_NEW}`)) {
      const id = path.slice(path.lastIndexOf("/") + 1);
      const nodes = Array.from(htmlOf[id].matchAll(/<(\w+)[^>]*data-node-id="(\w+)"/g), (m) =>
        node(m[2], m[1] === "h1" ? "heading" : m[1] === "p" ? "paragraph" : "section", m[2] === "s0000001" ? null : "s0000001"),
      );
      return json(200, {
        version: { ...versionItem(id, HTML_DOC, id === HTML_BASE ? CS_MERGED : CS_ID), design_system_version_id: null, nodes },
      });
    }
    return json(404, { error: "not_found" });
  });
  vi.stubGlobal("fetch", fetch);
  return api;
}

/** A WebSocket that stays connecting: the review needs no live events. */
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

// Frames --------------------------------------------------------------------------------

type Show = {
  type: "viewer.show";
  html: string;
  changes?: { node: string; change: string }[];
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

/** The `viewer.show` messages the page posted to a frame's window. */
function showsTo(target: Window): Show[] {
  return (posted.get(target) ?? []).filter(
    (message): message is Show => (message as { type?: string } | null)?.type === "viewer.show",
  );
}

// Lookups -------------------------------------------------------------------------------

async function findReview() {
  return screen.findByRole("heading", { name: new RegExp(CS_TITLE, "i") }, { timeout: 3000 });
}

async function findSection(name: RegExp) {
  return screen.findByRole("region", { name }, { timeout: 3000 });
}

/** The entry of the doc titled `title` in the Docs section. */
async function findEntry(title: string): Promise<HTMLElement> {
  const docs = await findSection(/^(docs|changes|changed docs)$/i);
  return waitFor(() => {
    const named = within(docs).getAllByText(title)[0];
    const entry = named.closest("li, article, [role=listitem]");
    expect(entry, `the entry of ${title} is a list item or an article`).toBeTruthy();
    return entry as HTMLElement;
  });
}

/** The renderer that draws the canvas in `region`. */
async function rendererIn(region: HTMLElement) {
  return waitFor(() => {
    const canvas = region.querySelector("canvas");
    expect(canvas, "the region has a canvas").toBeTruthy();
    const instance = fake.instances.find((i) => i.canvas === canvas && !i.destroyed);
    expect(instance, "a renderer draws the region's canvas").toBeTruthy();
    expect(instance!.shown.length, "the renderer was shown a page").toBeGreaterThan(0);
    return instance!;
  });
}

function lastShownPage(instance: { shown: { page: unknown }[] }) {
  return instance.shown[instance.shown.length - 1].page as JsonNode;
}

function findNode(nodes: JsonNode[], id: string): JsonNode | null {
  for (const candidate of nodes) {
    if (candidate.id === id) return candidate;
    const below = findNode(candidate.children ?? [], id);
    if (below) return below;
  }
  return null;
}

type Highlight = { node: string | null; change: string | null; box: Box };

/** The highlights over a region's canvas. */
function highlightsIn(region: HTMLElement): Highlight[] {
  return Array.from(region.querySelectorAll<HTMLElement>("[data-change]")).map((element) => ({
    node: element.getAttribute("data-node"),
    change: element.getAttribute("data-change"),
    box: {
      x: parseFloat(element.style.left),
      y: parseFloat(element.style.top),
      width: parseFloat(element.style.width),
      height: parseFloat(element.style.height),
    },
  }));
}

/** Whether a highlight's box covers `box`, with a few pixels of outline allowed. */
function covers(highlight: Box, box: Box) {
  return (
    Math.abs(highlight.x - box.x) <= 4 &&
    Math.abs(highlight.y - box.y) <= 4 &&
    Math.abs(highlight.width - box.width) <= 8 &&
    Math.abs(highlight.height - box.height) <= 8
  );
}

/** The highlight of `node`, failing when there is none. */
function highlightOf(region: HTMLElement, address: string): Highlight {
  const found = highlightsIn(region).find((h) => h.node === address);
  expect(found, `the node ${address} is highlighted: ${JSON.stringify(highlightsIn(region))}`).toBeTruthy();
  return found!;
}

/** The Approve button: the change set's only button named "Approve" or "Approve change set". */
async function findApprove() {
  return waitFor(() => {
    const buttons = screen.getAllByRole("button", { name: /^approve(\s+(the\s+)?change\s*set)?$/i });
    expect(buttons.length, "one Approve button").toBe(1);
    return buttons[0] as HTMLButtonElement;
  });
}

function rejectButtons() {
  return screen.queryAllByRole("button", { name: /^reject\b/i }) as HTMLButtonElement[];
}

/** The text that describes an element: its title and what aria-describedby points to. */
function descriptionOf(element: HTMLElement) {
  const ids = (element.getAttribute("aria-describedby") ?? "").split(/\s+/).filter(Boolean);
  const parts = ids.map((id) => document.getElementById(id)?.textContent ?? "");
  return [element.getAttribute("title") ?? "", ...parts].join(" ");
}

/** The entry of the decision with `direction` in the Decisions section. */
async function findDecision(direction: string): Promise<HTMLElement> {
  const decisions = await findSection(/^decisions$/i);
  return waitFor(() => {
    const named = within(decisions).getByText(direction);
    return (named.closest("li, article, tr, [role=listitem], [role=row]") ?? named.parentElement) as HTMLElement;
  });
}

function postsTo(path: string) {
  return api.calls.filter((call) => call.method === "POST" && call.path === path);
}

let api: ReturnType<typeof fakeApi>;

beforeEach(() => {
  fake.instances.length = 0;
  fake.boxes.clear();
  seedBoxes();
  api = fakeApi({ undecided: true });
  posted.clear();
  watchFrames();
  vi.stubGlobal("WebSocket", FakeWebSocket);
  window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
});

afterEach(() => {
  observer?.disconnect();
  observer = null;
  cleanup();
  vi.unstubAllGlobals();
});

// The page ------------------------------------------------------------------------------

describe("the change set review page", () => {
  test("the project home links each change set's title to its review", async () => {
    const router = renderAt(HOME_PATH);
    const changesets = await findSection(/^change sets$/i);
    const link = await within(changesets).findByRole("link", { name: new RegExp(CS_TITLE, "i") });
    expect(link.getAttribute("href")).toBe(REVIEW_PATH);
    fireEvent.click(link);
    await findReview();
    expect(router.state.location.pathname).toBe(REVIEW_PATH);
  });

  test("it lists each doc the change set changes with a summary of its node changes", async () => {
    renderAt(REVIEW_PATH);
    await findReview();
    const design = await findEntry(DESIGN_TITLE);
    const html = await findEntry(HTML_TITLE);
    expect(design, "each doc has its own entry").not.toBe(html);
    await waitFor(() => {
      const said = design.textContent ?? "";
      expect(said).toMatch(/(^|\D)1\s*added/i);
      expect(said).toMatch(/(^|\D)2\s*changed/i);
      expect(said).toMatch(/(^|\D)1\s*removed/i);
    });
    await waitFor(() => {
      const said = html.textContent ?? "";
      expect(said).toMatch(/(^|\D)1\s*added/i);
      expect(said).toMatch(/(^|\D)1\s*changed/i);
      expect(said).toMatch(/(^|\D)1\s*removed/i);
      expect(said).not.toMatch(/(^|\D)2\s*changed/i);
    });
  });
});

describe("boards", () => {
  test("a design shows the base and the new version side by side, on the page that holds the first change", async () => {
    renderAt(REVIEW_PATH);
    const entry = await findEntry(DESIGN_TITLE);
    const base = await within(entry).findByRole("region", { name: /^base\b/i }, { timeout: 3000 });
    const after = await within(entry).findByRole("region", { name: /^new version\b/i }, { timeout: 3000 });
    const baseRenderer = await rendererIn(base);
    const newRenderer = await rendererIn(after);
    expect(baseRenderer, "each side has its own renderer").not.toBe(newRenderer);

    await waitFor(() => {
      const before = lastShownPage(baseRenderer);
      const now = lastShownPage(newRenderer);
      expect(before.id, "the base shows the Mobile page").toBe(MOBILE);
      expect(now.id, "the new version shows the Mobile page").toBe(MOBILE);
      expect(findNode([before], TOTAL)?.content, "the base as the base version has it").toBe("Total 96 kr");
      expect(findNode([before], BUY), "the base still has Buy").toBeTruthy();
      expect(findNode([now], TOTAL)?.content, "the new version as the change set has it").toBe("Total 120 kr");
      expect(findNode([now], NOTE), "the new version has Note").toBeTruthy();
    });
  });

  test("added and changed nodes are highlighted on the new version, removed ones on the base, each on its box", async () => {
    renderAt(REVIEW_PATH);
    const entry = await findEntry(DESIGN_TITLE);
    const base = await within(entry).findByRole("region", { name: /^base\b/i }, { timeout: 3000 });
    const after = await within(entry).findByRole("region", { name: /^new version\b/i }, { timeout: 3000 });
    await rendererIn(base);
    await rendererIn(after);

    await waitFor(() => {
      const added = highlightOf(after, NOTE);
      expect(added.change).toBe("added");
      expect(covers(added.box, { x: 420, y: 40, width: 100, height: 20 }), `Note's highlight: ${JSON.stringify(added.box)}`).toBe(true);
      const changed = highlightOf(after, TOTAL);
      expect(changed.change).toBe("changed");
      expect(covers(changed.box, { x: 20, y: 100, width: 80, height: 18 }), `Total's highlight: ${JSON.stringify(changed.box)}`).toBe(true);
      const removed = highlightOf(base, BUY);
      expect(removed.change).toBe("removed");
      expect(covers(removed.box, { x: 20, y: 20, width: 120, height: 40 }), `Buy's highlight: ${JSON.stringify(removed.box)}`).toBe(true);
    });

    const unchanged = [CART_BOARD, RECEIPT, SUMMARY];
    for (const region of [base, after]) {
      const marked = highlightsIn(region).map((h) => h.node);
      for (const address of unchanged) {
        expect(marked, `the unchanged node ${address} is not highlighted`).not.toContain(address);
      }
    }
    expect(highlightsIn(base).map((h) => h.node), "an added node is not marked on the base").not.toContain(NOTE);
    expect(highlightsIn(after).map((h) => h.node), "a removed node is not marked on the new version").not.toContain(BUY);
  });

  test("the highlights follow the view when the reviewer zooms and pans", async () => {
    renderAt(REVIEW_PATH);
    const entry = await findEntry(DESIGN_TITLE);
    const after = await within(entry).findByRole("region", { name: /^new version\b/i }, { timeout: 3000 });
    const renderer = await rendererIn(after);
    await waitFor(() => highlightOf(after, NOTE));

    act(() => {
      renderer.view = { x: 10, y: 20, zoom: 2 };
      for (const listener of Array.from(renderer.viewListeners)) listener();
    });
    await waitFor(() => {
      const moved = highlightOf(after, NOTE);
      expect(covers(moved.box, { x: 820, y: 40, width: 200, height: 40 }), `Note's highlight after the zoom: ${JSON.stringify(moved.box)}`).toBe(true);
    });
  });
});

describe("HTML docs", () => {
  test("the doc's frame shows the version under review with its added and changed blocks marked", async () => {
    renderAt(REVIEW_PATH);
    const entry = await findEntry(HTML_TITLE);
    await waitFor(() => expect(entry.querySelectorAll("iframe").length, "the entry shows the doc in a frame").toBeGreaterThan(0));
    for (const frame of Array.from(entry.querySelectorAll("iframe"))) {
      expect(frame.getAttribute("sandbox"), "the viewer frame is sandboxed as in US-042").toBe("allow-scripts");
    }

    const show = await waitFor(
      () => {
        const frames = Array.from(entry.querySelectorAll("iframe"));
        for (const frame of frames) {
          const target = frame.contentWindow!;
          if (!showsTo(target).length) {
            window.dispatchEvent(new MessageEvent("message", { data: { type: "viewer.ready" }, origin: "null", source: target }));
          }
        }
        const latest = frames
          .map((frame) => showsTo(frame.contentWindow!))
          .map((shows) => shows[shows.length - 1])
          .find((message) => message?.html === HTML_NEW_HTML);
        expect(latest, "a frame shows the version under review").toBeTruthy();
        return latest!;
      },
      { timeout: 3000 },
    );

    const changes = show.changes ?? [];
    expect(changes).toEqual(
      expect.arrayContaining([
        { node: "p0000004", change: "added" },
        { node: "p0000001", change: "changed" },
      ]),
    );
    const marked = changes.map((change) => change.node);
    for (const untouched of ["s0000001", "h0000001", "p0000002"]) {
      expect(marked, `the unchanged block ${untouched} is not marked`).not.toContain(untouched);
    }
  });
});

describe("linked decisions", () => {
  test("each linked decision is shown with its status", async () => {
    renderAt(REVIEW_PATH);
    await findReview();
    const approved = await findDecision(APPROVED_DIRECTION);
    const proposed = await findDecision(PROPOSED_DIRECTION);
    expect(within(approved).queryAllByText(/\bapproved\b/i).length, "the approved decision says approved").toBeGreaterThan(0);
    expect(within(proposed).queryAllByText(/\bproposed\b/i).length, "the proposed decision says proposed").toBeGreaterThan(0);
  });

  test("Approve is disabled while a linked decision is not approved, and names that decision", async () => {
    renderAt(REVIEW_PATH);
    await findReview();
    const approve = await findApprove();
    await waitFor(() => expect(approve.disabled, "Approve is disabled").toBe(true));
    const said = descriptionOf(approve);
    expect(said, "the description names the decision that is not approved").toContain(PROPOSED_DIRECTION);
    expect(said, "and not the approved one").not.toContain(APPROVED_DIRECTION);
    fireEvent.click(approve);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(postsTo(`${API}/changesets/${CS_ID}/approve`), "nothing is sent").toHaveLength(0);
  });
});

describe("approving and rejecting", () => {
  test("with every linked decision approved, Approve approves the whole change set", async () => {
    api = fakeApi({ undecided: false });
    renderAt(REVIEW_PATH);
    await findReview();
    expect(screen.queryAllByText(/\bmerged\b/i), "nothing is merged yet").toHaveLength(0);
    const approve = await findApprove();
    await waitFor(() => expect(approve.disabled, "Approve is enabled").toBe(false));
    fireEvent.click(approve);
    await waitFor(() => expect(postsTo(`${API}/changesets/${CS_ID}/approve`)).toHaveLength(1));
    await waitFor(() => expect(screen.queryAllByText(/\bmerged\b/i).length, "the page shows the change set merged").toBeGreaterThan(0));
  });

  test("rejecting the change set requires a reason", async () => {
    renderAt(REVIEW_PATH);
    await findReview();
    expect(screen.queryAllByText(/\brejected\b/i), "nothing is rejected yet").toHaveLength(0);
    await waitFor(() => expect(rejectButtons().length).toBeGreaterThan(0));
    fireEvent.click(rejectButtons()[0]);
    const reason = await screen.findByRole("textbox", { name: /reason/i });
    const confirm = () => {
      const buttons = rejectButtons();
      return buttons[buttons.length - 1];
    };
    fireEvent.click(confirm());
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(postsTo(`${API}/changesets/${CS_ID}/reject`), "nothing is sent without a reason").toHaveLength(0);

    fireEvent.change(reason, { target: { value: "The receipt copy is off-brand." } });
    await waitFor(() => expect(confirm().disabled).toBe(false));
    fireEvent.click(confirm());
    await waitFor(() => expect(postsTo(`${API}/changesets/${CS_ID}/reject`)).toHaveLength(1));
    expect(postsTo(`${API}/changesets/${CS_ID}/reject`)[0].body).toEqual({ reason: "The receipt copy is off-brand." });
    await waitFor(() => expect(screen.queryAllByText(/\brejected\b/i).length, "the page shows the change set rejected").toBeGreaterThan(0));
  });
});

describe("conflicts", () => {
  test("a panel lists the conflicts from rebases and how each was resolved", async () => {
    renderAt(REVIEW_PATH);
    await findReview();
    const panel = await findSection(/^conflicts$/i);
    await waitFor(() => {
      expect(within(panel).queryAllByText(/p0000001/).length, "the resolved conflict's node").toBeGreaterThan(0);
      expect(within(panel).queryAllByText(/p0000004/).length, "the open conflict's node").toBeGreaterThan(0);
    });
    const resolved = within(panel).getAllByText(/p0000001/)[0].closest("li, article, tr, [role=listitem], [role=row]");
    const open = within(panel).getAllByText(/p0000004/)[0].closest("li, article, tr, [role=listitem], [role=row]");
    expect(resolved, "a conflict is a list item, an article or a row").toBeTruthy();
    expect(open, "a conflict is a list item, an article or a row").toBeTruthy();
    expect(resolved).not.toBe(open);
    expect(resolved!.textContent, "the resolved conflict says how it was resolved").toContain(RESOLVED_NOTE);
    expect(open!.textContent, "the open conflict says it is unresolved").toMatch(/unresolved|not resolved/i);
    expect(open!.textContent).not.toContain(RESOLVED_NOTE);
  });
});
