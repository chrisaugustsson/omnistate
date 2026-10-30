// @vitest-environment jsdom
// US-114: a doc's Activity tab (/…/docs/<doc>/activity) and one write in it
// (/…/docs/<doc>/activity/<seq>), over the US-111 activity routes. The app is
// rendered with its own router on a memory history, fetch is replaced by a fake
// of the API and the renderer by a fake that records what it is asked to draw.
// jsdom does not run a frame's `srcdoc`, so the test plays each frame: it says
// `viewer.ready` and records the `viewer.show` the page posts to it.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const fake = vi.hoisted(() => ({
  /** The tags of every node the renderer was asked to draw (`showTags`). */
  tagsShown: [] as string[],
  /** The component of every instance the renderer was asked to draw (`show`). */
  componentsShown: [] as unknown[],
}));

vi.mock("@/design/renderer", () => ({
  createDesignRenderer: async () => ({
    showTags(tags: string) {
      fake.tagsShown.push(tags);
    },
    show(page: { children: { children: { component: unknown }[] }[] }, system: { $components?: { id: unknown }[] }) {
      const placed = page.children[0].children[0].component;
      fake.componentsShown.push(system.$components?.find((component) => component.id === placed));
    },
    visibleBoards() {
      return [];
    },
    destroy() {},
  }),
}));

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const DOC_ID = "0192d7a0-5555-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const DOC_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/docs/${DOC_ID}`;
const ACTIVITY_PATH = `${DOC_PATH}/activity`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T10:00:00Z";
const WRITER = { run: "run-1", role: "writer", model: "claude-opus-4", harness: "claude-code" };

function doc(kind: string, title: string) {
  return {
    id: DOC_ID,
    project_id: PROJECT_ID,
    feature_id: null,
    kind,
    pillar: kind === "html" ? "spec" : "ui",
    title,
    created_at: AT,
    drafts: [],
    claims: [],
  };
}

function entry(seq: number, summary: string, kind = "edit") {
  return { seq, kind, summary, agent: WRITER, created_at: AT };
}

type Doc = ReturnType<typeof doc>;
type Routes = Record<string, unknown>;

/**
 * A fake of the API: the auth routes, the project, the doc and the answers in
 * `routes`, by path with its query; every other request is a 404.
 */
function fakeApi(shown: Doc, routes: Routes) {
  const requested: string[] = [];
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(typeof input === "string" ? input : input instanceof URL ? input.href : input.url, "http://localhost:3000");
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    const at = `${url.pathname}${url.search}`;
    requested.push(at);
    if (method !== "GET") return json(404, { error: "not_found" });
    switch (url.pathname) {
      case "/api/v1/auth/me":
        return json(200, { user: USER });
      case "/api/v1/auth/workspaces":
        return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
      case `/api/v1/w/${WORKSPACE_ID}/projects`:
        return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
      case `${API}/home`:
        return json(200, {
          project_id: PROJECT_ID,
          app: { last_activity: null },
          features: [],
          docs: [shown],
          threads: [],
          waiting: { comments: [] },
        });
      case `${API}/docs/${DOC_ID}`:
        return json(200, { doc: shown });
      default:
        break;
    }
    return at in routes ? json(200, routes[at]) : json(404, { error: "not_found" });
  });
  vi.stubGlobal("fetch", fetch);
  return requested;
}

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

// The frames ---------------------------------------------------------------------------

/** What the page posted to each frame's window. */
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

/** The HTML the page last showed in `frame`; until it has shown any, the test says `viewer.ready` as the frame. */
function shownIn(frame: HTMLIFrameElement): string {
  const target = frame.contentWindow!;
  const shows = (posted.get(target) ?? []).filter(
    (message): message is { type: string; html: string } => (message as { type?: string })?.type === "viewer.show",
  );
  if (!shows.length) window.dispatchEvent(new MessageEvent("message", { data: { type: "viewer.ready" }, origin: "null", source: target }));
  expect(shows.length, "the page showed the frame something").toBeGreaterThan(0);
  return shows[shows.length - 1].html;
}

/** The Before or After column of a pair. */
function side(pair: HTMLElement, caption: "Before" | "After") {
  const label = within(pair).getByText(caption);
  return label.parentElement as HTMLElement;
}

describe("a doc's activity", () => {
  beforeEach(() => {
    fake.tagsShown.length = 0;
    fake.componentsShown.length = 0;
    vi.stubGlobal("scrollTo", () => {});
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
      () => ({ drawImage() {} }) as unknown as CanvasRenderingContext2D,
    );
    watchFrames();
  });

  afterEach(() => {
    cleanup();
    observer?.disconnect();
    posted.clear();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  test("the doc's viewer has an Activity tab that opens the list of writes", async () => {
    fakeApi(doc("html", "Checkout PRD"), {
      [`${API}/docs/${DOC_ID}/html`]: { doc_id: DOC_ID, html: '<p data-omni-id="p0000001">Ship in a day.</p>' },
      [`${API}/comments?doc=${DOC_ID}`]: { project_id: PROJECT_ID, comments: [] },
      [`${API}/docs/${DOC_ID}/activity`]: { entries: [entry(1, "Created the doc", "create")], next_before: null },
    });
    const router = renderAt(DOC_PATH);
    await waitFor(() => expect(document.querySelector("iframe")).toBeTruthy());

    const tabs = screen.getByRole("navigation", { name: "Doc views" });
    fireEvent.click(within(tabs).getByRole("link", { name: "Activity" }));

    await waitFor(() => expect(router.state.location.pathname).toBe(ACTIVITY_PATH));
    expect(await screen.findByRole("link", { name: /Created the doc/ })).toBeTruthy();
  });

  test("the list opens each write at its seq, and Load more brings the next page", async () => {
    fakeApi(doc("html", "Checkout PRD"), {
      [`${API}/docs/${DOC_ID}/activity`]: { entries: [entry(3, "Changed the lead"), entry(2, "Added a table")], next_before: 2 },
      [`${API}/docs/${DOC_ID}/activity?before=2`]: { entries: [entry(1, "Created the doc", "create")], next_before: null },
    });
    renderAt(ACTIVITY_PATH);
    const list = await screen.findByRole("region", { name: "Activity" });

    const lead = await within(list).findByRole("link", { name: /Changed the lead/ });
    expect(lead.getAttribute("href")).toBe(`${ACTIVITY_PATH}/3`);
    fireEvent.click(within(list).getByRole("button", { name: "Load more" }));

    await waitFor(() =>
      expect(within(list).getAllByRole("link").map((link) => link.getAttribute("href"))).toEqual([
        `${ACTIVITY_PATH}/3`,
        `${ACTIVITY_PATH}/2`,
        `${ACTIVITY_PATH}/1`,
      ]),
    );
    expect(within(list).queryByRole("button", { name: "Load more" }), "the last page has been read").toBeNull();
  });

  test("an HTML write shows each changed element before and after in sandboxed frames with the doc's own style, an inserted one Not present before and a deleted one Not present after", async () => {
    const style = "<style>.lead{color:teal}</style>";
    fakeApi(doc("html", "Checkout PRD"), {
      [`${API}/docs/${DOC_ID}/html`]: {
        doc_id: DOC_ID,
        html: `<div data-omni-id="s0000001">${style}</div><p data-omni-id="p0000001" class="lead">Ship in two days.</p>`,
      },
      [`${API}/docs/${DOC_ID}/activity/4`]: {
        ...entry(4, "Changed the lead"),
        items: [
          { node: "p0000001", before: '<p data-omni-id="p0000001" class="lead">Ship in a day.</p>', after: '<p data-omni-id="p0000001" class="lead">Ship in two days.</p>' },
          { node: "p0000002", before: null, after: '<p data-omni-id="p0000002">Returns are free.</p>' },
          { node: "p0000003", before: '<p data-omni-id="p0000003">Gone.</p>', after: null },
        ],
      },
    });
    renderAt(`${ACTIVITY_PATH}/4`);

    const changed = await screen.findByRole("region", { name: "Element p0000001" });
    const inserted = await screen.findByRole("region", { name: "Element p0000002" });
    const deleted = await screen.findByRole("region", { name: "Element p0000003" });
    const frames = Array.from(document.querySelectorAll("iframe"));
    expect(frames).toHaveLength(4);
    expect(frames.map((frame) => frame.getAttribute("sandbox"))).toEqual(["allow-scripts", "allow-scripts", "allow-scripts", "allow-scripts"]);

    const before = side(changed, "Before").querySelector("iframe")!;
    const after = side(changed, "After").querySelector("iframe")!;
    await waitFor(() => {
      expect(shownIn(before)).toContain("Ship in a day.");
      expect(shownIn(before)).toContain(style);
      expect(shownIn(after)).toContain("Ship in two days.");
      expect(shownIn(after)).toContain(style);
    });
    expect(side(inserted, "Before").textContent).toContain("Not present");
    expect(side(inserted, "Before").querySelector("iframe")).toBeNull();
    await waitFor(() => expect(shownIn(side(inserted, "After").querySelector("iframe")!)).toContain("Returns are free."));
    expect(side(deleted, "After").textContent).toContain("Not present");
    expect(side(deleted, "After").querySelector("iframe")).toBeNull();
    await waitFor(() => expect(shownIn(side(deleted, "Before").querySelector("iframe")!)).toContain("Gone."));
  });

  test("a rename shows the old and the new title as plain text, without frames", async () => {
    fakeApi(doc("html", "<b>Checkout</b> & returns"), {
      [`${API}/docs/${DOC_ID}/activity/5`]: {
        ...entry(5, "Renamed the doc", "rename"),
        items: [],
        title: { before: "<b>Checkout</b> PRD", after: "<b>Checkout</b> & returns" },
      },
    });
    renderAt(`${ACTIVITY_PATH}/5`);

    const title = await screen.findByRole("region", { name: "Title" });
    expect(side(title, "Before").textContent).toContain("<b>Checkout</b> PRD");
    expect(side(title, "After").textContent).toContain("<b>Checkout</b> & returns");
    expect(title.querySelector("b")).toBeNull();
    expect(document.querySelector("iframe")).toBeNull();
  });

  test("a design write draws each changed board as it was before and after, an added one Not present before and a removed one Not present after", async () => {
    const before = '<Board id="b0000001" name="Cart, old" width={390} height={844}/>';
    const after = '<Board id="b0000001" name="Cart, new" width={390} height={844}/>';
    const added = '<Board id="b0000002" name="Receipt" width={390} height={844}/>';
    const removed = '<Board id="b0000003" name="Coupon" width={390} height={844}/>';
    const board = (id: string, tags: string) => ({ board_id: id, doc_id: DOC_ID, page_id: "p0000001", node: {}, tags });
    const requested = fakeApi(doc("design", "Cart screens"), {
      [`${API}/docs/${DOC_ID}/activity/7`]: {
        ...entry(7, "Changed the cart"),
        items: [
          { board: "b0000001", before_at: 6, after_at: 7 },
          { board: "b0000002", before_at: null, after_at: 7 },
          { board: "b0000003", before_at: 6, after_at: null },
        ],
      },
      [`${API}/docs/${DOC_ID}/boards/b0000001?at=6`]: board("b0000001", before),
      [`${API}/docs/${DOC_ID}/boards/b0000001?at=7`]: board("b0000001", after),
      [`${API}/docs/${DOC_ID}/boards/b0000002?at=7`]: board("b0000002", added),
      [`${API}/docs/${DOC_ID}/boards/b0000003?at=6`]: board("b0000003", removed),
    });
    renderAt(`${ACTIVITY_PATH}/7`);

    const cart = await screen.findByRole("region", { name: "Board b0000001" });
    const receipt = await screen.findByRole("region", { name: "Board b0000002" });
    const coupon = await screen.findByRole("region", { name: "Board b0000003" });
    await waitFor(() => expect(within(side(cart, "Before")).getByRole("img", { name: "Picture of Cart, old" })).toBeTruthy());
    expect(within(side(cart, "After")).getByRole("img", { name: "Picture of Cart, new" })).toBeTruthy();
    await waitFor(() => expect(within(side(receipt, "After")).getByRole("img", { name: "Picture of Receipt" })).toBeTruthy());
    await waitFor(() => expect(within(side(coupon, "Before")).getByRole("img", { name: "Picture of Coupon" })).toBeTruthy());
    expect(fake.tagsShown).toEqual(expect.arrayContaining([before, after, added, removed]));
    expect(side(receipt, "Before").textContent).toContain("Not present");
    expect(within(side(receipt, "Before")).queryByRole("img")).toBeNull();
    expect(requested.filter((path) => path.includes("/boards/b0000002"))).toEqual([`${API}/docs/${DOC_ID}/boards/b0000002?at=7`]);
    expect(side(coupon, "After").textContent).toContain("Not present");
    expect(within(side(coupon, "After")).queryByRole("img")).toBeNull();
    expect(requested.filter((path) => path.includes("/boards/b0000003"))).toEqual([`${API}/docs/${DOC_ID}/boards/b0000003?at=6`]);
  });

  test("a design-system write draws each changed component at before_at and after_at, a removed one Not present after, and the tokens as formatted JSON", async () => {
    const oldButton = { id: "c0000001", type: "component", name: "Button, old", children: [] };
    const newButton = { id: "c0000001", type: "component", name: "Button, new", children: [] };
    const badge = { id: "c0000002", type: "component", name: "Badge", children: [] };
    const oldTokens = { color: { brand: { $type: "color", $value: "#0f766e" } } };
    const newTokens = { color: { brand: { $type: "color", $value: "#be185d" } } };
    const component = (node: Record<string, unknown>, at: number) => ({ component_id: "c0000001", doc_id: DOC_ID, at, node, tags: "" });
    const requested = fakeApi(doc("design-system", "Tokens"), {
      [`${API}/docs/${DOC_ID}/html`]: { doc_id: DOC_ID, html: JSON.stringify({ ...newTokens, $components: [newButton] }) },
      [`${API}/docs/${DOC_ID}/activity/9`]: {
        ...entry(9, "Changed the button and the brand colour"),
        items: [
          { component: "c0000001", before_at: 8, after_at: 9 },
          { component: "c0000002", before_at: 8, after_at: null },
        ],
        tokens: { before: oldTokens, after: newTokens },
      },
      [`${API}/components/c0000001?at=8`]: component(oldButton, 8),
      [`${API}/components/c0000001?at=9`]: component(newButton, 9),
      [`${API}/components/c0000002?at=8`]: { ...component(badge, 8), component_id: "c0000002" },
    });
    renderAt(`${ACTIVITY_PATH}/9`);

    const button = await screen.findByRole("region", { name: "Component c0000001" });
    await waitFor(() => expect(within(side(button, "Before")).getByRole("img", { name: "Picture of Button, old" })).toBeTruthy());
    await waitFor(() => expect(within(side(button, "After")).getByRole("img", { name: "Picture of Button, new" })).toBeTruthy());
    expect(requested).toEqual(expect.arrayContaining([`${API}/components/c0000001?at=8`, `${API}/components/c0000001?at=9`]));
    expect(fake.componentsShown).toEqual(expect.arrayContaining([oldButton, newButton]));
    const removed = screen.getByRole("region", { name: "Component c0000002" });
    await waitFor(() => expect(within(side(removed, "Before")).getByRole("img", { name: "Picture of Badge" })).toBeTruthy());
    expect(side(removed, "After").textContent).toContain("Not present");
    expect(within(side(removed, "After")).queryByRole("img")).toBeNull();
    expect(requested.filter((path) => path.includes("/components/c0000002"))).toEqual([`${API}/components/c0000002?at=8`]);

    const tokens = screen.getByRole("region", { name: "Tokens" });
    const json = (caption: "Before" | "After") => side(tokens, caption).querySelector("pre")!.textContent!;
    expect(JSON.parse(json("Before"))).toEqual(oldTokens);
    expect(JSON.parse(json("After"))).toEqual(newTokens);
    expect(json("After")).toContain('\n  "color": {\n');
  });

  test("a design system's creation shows Not present for the tokens before", async () => {
    const tokens = { color: { brand: { $type: "color", $value: "#0f766e" } } };
    fakeApi(doc("design-system", "Tokens"), {
      [`${API}/docs/${DOC_ID}/activity/1`]: {
        ...entry(1, "Created the design system", "create"),
        items: [],
        tokens: { before: null, after: tokens },
      },
    });
    renderAt(`${ACTIVITY_PATH}/1`);

    const shown = await screen.findByRole("region", { name: "Tokens" });
    expect(side(shown, "Before").textContent).toContain("Not present");
    expect(side(shown, "Before").querySelector("pre")).toBeNull();
    expect(JSON.parse(side(shown, "After").querySelector("pre")!.textContent!)).toEqual(tokens);
  });

  test("a board pair is drawn only once it scrolls into view", async () => {
    const observed: { callback: IntersectionObserverCallback; element: Element }[] = [];
    vi.stubGlobal(
      "IntersectionObserver",
      class {
        constructor(readonly callback: IntersectionObserverCallback) {}
        observe(element: Element) {
          observed.push({ callback: this.callback, element });
        }
        disconnect() {}
        unobserve() {}
      },
    );
    const tags = '<Board id="b0000001" name="Cart" width={390} height={844}/>';
    const requested = fakeApi(doc("design", "Cart screens"), {
      [`${API}/docs/${DOC_ID}/activity/2`]: {
        ...entry(2, "Added the cart", "create"),
        items: [{ board: "b0000001", before_at: null, after_at: 2 }],
      },
      [`${API}/docs/${DOC_ID}/boards/b0000001?at=2`]: { board_id: "b0000001", doc_id: DOC_ID, page_id: "p0000001", node: {}, tags },
    });
    renderAt(`${ACTIVITY_PATH}/2`);

    const cart = await screen.findByRole("region", { name: "Board b0000001" });
    await waitFor(() => expect(observed).toHaveLength(1));
    expect(within(cart).queryByRole("img"), "nothing drawn while out of view").toBeNull();
    expect(requested.filter((path) => path.includes("/boards/"))).toEqual([]);

    const [{ callback, element }] = observed;
    callback([{ isIntersecting: true, target: element } as IntersectionObserverEntry], {} as IntersectionObserver);

    await waitFor(() => expect(within(cart).getByRole("img", { name: "Picture of Cart" })).toBeTruthy());
    expect(requested.filter((path) => path.includes("/boards/"))).toEqual([`${API}/docs/${DOC_ID}/boards/b0000001?at=2`]);
  });
});
