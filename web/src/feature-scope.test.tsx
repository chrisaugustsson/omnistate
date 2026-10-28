// @vitest-environment jsdom
// US-065: feature scope in reviews. A feature's change set can change its own
// docs and the project's global docs (FR-54), so the reviewer has to see when
// it reaches beyond the feature before approving it. Since US-110 a feature has
// no status, so nobody marks it done or reopens it. The app is rendered with its own
// router on a memory history, the browser's fetch is replaced by a fake of the
// API, and the browser's WebSocket by a fake socket that never opens, so a page
// shows what it loads itself.
//
// The fake serves the project's home (its docs, change sets and threads with
// their `feature_id`, and `features`), the features, every doc (GET .../docs and
// GET .../docs/<doc>, each DocBody with its `feature_id`) and each change set's
// review. A reviewed doc also carries its `feature_id`; the page may learn a
// doc's scope from either.
//
// The change set review (/w/<workspace>/projects/<project>/changesets/<id>):
//
// - names its scope: a feature's change set links its feature by name to the
//   feature's overview, .../features/<feature>; a project-level change set
//   names the project and links no feature.
// - in the entry (a list item or an article) of each doc of its "Docs" section,
//   a feature's change set marks every global doc with a badge reading
//   "Global"; the feature's own docs have none.
// - its merge box says "Changes N global docs" ahead of the Approve button,
//   N being the global docs the feature's change set changes.
//
// The thread page and the doc and design viewers show where the item lives, a
// navigation named "Breadcrumb": the project's name linking to the project's
// overview, then the feature's name linking to the feature's overview (only
// for an item of a feature), then the section ("Threads" or "Docs"; as a link,
// it goes to that section of the item's feature, or of the project).
//
// US-110: a feature has no status. Its overview has no "Mark done" and no
// "Reopen" button, no page of a feature (its overview, its Docs, Change sets
// and Threads) shows a "Done" badge, and the sidebar lists every feature with
// no "Done" toggle and no Done badge. The fake API has no done or reopen
// route.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

// The design viewer draws on a canvas with the WASM renderer; these tests look
// at what is around it only.
vi.mock("@/design/renderer", () => ({
  createDesignRenderer: async () => ({
    show() {},
    layers() {
      return [];
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

function featurePath(featureId: string, section = "") {
  return `${PROJECT_PATH}/features/${featureId}${section ? `/${section}` : ""}`;
}

/**
 * A feature as the API sends it. `legacyStatus` is the `status` field that
 * features had before US-110: the fixtures keep it so that they parse against
 * the API schema before and after the field goes, and so that the tests also
 * show that the app pays no heed to it.
 */
function feature(id: string, name: string, description: string, legacyStatus: string) {
  return { id, project_id: PROJECT_ID, name, description, status: legacyStatus, created_at: AT };
}

function doc(featureId: string | null, kind: string, pillar: string, title: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: featureId,
    kind,
    pillar,
    title,
    created_at: AT,
    mainline_version_id: null,
    drafts: [],
    claims: [],
  };
}

type Doc = ReturnType<typeof doc>;

function changeset(featureId: string | null, title: string, status: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: featureId,
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

function thread(on: { id: string; feature_id: string | null }, question: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: on.feature_id,
    changeset_id: on.id,
    question,
    criteria: ["Readable at 320 px"],
    run_tag: "run-1",
    created_at: AT,
    phases: [{ number: 1, sealed: false, started_at: AT, posted_by: [] }],
    decision: null,
  };
}

const HTML = '<section data-omni-id="s0000001"><p data-omni-id="p0000001">Saved items.</p></section>';

/** A doc as the review lists it: a changed doc, or a new one when `fresh`. */
function reviewed(of: Doc, fresh = false) {
  return {
    doc_id: of.id,
    feature_id: of.feature_id,
    kind: of.kind,
    pillar: of.pillar,
    title: of.title,
    base_version_id: fresh ? null : newId(),
    version_id: newId(),
    removed: false,
    changes: {
      added: fresh ? [{ id: "p0000001", type: "paragraph", parent: "s0000001", fingerprint: "fp-1" }] : [],
      removed: [],
      changed: fresh ? [] : [{ id: "p0000001", type: "paragraph", parent: "s0000001", fingerprint: "fp-2" }],
      moved: [],
    },
  };
}

/**
 * A project with two global docs (an HTML doc and the design system), a
 * feature (Wishlist) with an HTML doc and a design, and another feature
 * (Loyalty points, whose legacy status says done). The Wishlist's change set "Share saved lists" changes its
 * own PRD and both global docs; its "Draft saved items" changes its own PRD
 * only; the project-level "Rework the hero" changes the shop PRD. No doc's or
 * item's title holds the project's or a feature's name.
 */
function seeded() {
  const wishlist = feature(WISHLIST, "Wishlist", "Shoppers save products for later.", "active");
  const loyalty = feature(LOYALTY, "Loyalty points", "Earn points on every order.", "done");

  const shopPrd = doc(null, "html", "spec", "Shop PRD");
  const designSystem = doc(null, "design-system", "ui", "Design system");
  const savedPrd = doc(WISHLIST, "html", "spec", "Saved items PRD");
  const savedScreens = doc(WISHLIST, "design", "ui", "Saved items screens");
  const pointsPrd = doc(LOYALTY, "html", "spec", "Points PRD");

  const shareSaved = changeset(WISHLIST, "Share saved lists", "in_review");
  const draftSaved = changeset(WISHLIST, "Draft saved items", "in_review");
  const hero = changeset(null, "Rework the hero", "in_review");
  const ledger = changeset(LOYALTY, "Points ledger", "merged");

  const savedThread = thread(shareSaved, "Should saved lists be public?");
  const heroThread = thread(hero, "Which hero layout?");
  const pointsThread = thread(ledger, "When do points expire?");

  const home = {
    project_id: PROJECT_ID,
    docs: [shopPrd, designSystem, savedPrd, savedScreens, pointsPrd],
    changesets: [shareSaved, draftSaved, hero, ledger],
    threads: [savedThread, heroThread, pointsThread],
    waiting: { decisions: [], changesets: [shareSaved, draftSaved, hero], comments: [], questions: [] },
    features: [
      { feature: wishlist, docs: 2, open_changesets: 2, waiting: 2 },
      { feature: loyalty, docs: 1, open_changesets: 0, waiting: 0 },
    ],
  };
  const reviews = new Map<string, ReturnType<typeof reviewed>[]>([
    [shareSaved.id, [reviewed(savedPrd, true), reviewed(shopPrd), reviewed(designSystem)]],
    [draftSaved.id, [reviewed(savedPrd)]],
    [hero.id, [reviewed(shopPrd)]],
    [ledger.id, [reviewed(pointsPrd, true)]],
  ]);
  return {
    home,
    features: [wishlist, loyalty],
    reviews,
    docs: { savedPrd, savedScreens },
    changesets: { shareSaved, hero },
    threads: { savedThread, heroThread },
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
      case `GET ${API}/features`:
        return json(200, { features: seed.features });
      case `GET ${API}/changesets`:
        return json(200, { changesets: home.changesets });
      case `GET ${API}/docs`:
        return json(200, { docs: home.docs });
      case `GET ${API}/comments`:
        return json(200, { project_id: PROJECT_ID, comments: [] });
      default:
        break;
    }
    for (const f of seed.features) {
      if (method === "GET" && path === `${API}/features/${f.id}`) return json(200, { feature: f });
    }
    if (method !== "GET") return json(404, { error: "not_found" });
    for (const d of home.docs) {
      if (path === `${API}/docs/${d.id}`) return json(200, { doc: d });
      if (path === `${API}/docs/${d.id}/versions`) return json(200, { doc_id: d.id, versions: [] });
      if (path === `${API}/docs/${d.id}/html`) return json(200, { doc_id: d.id, html: HTML, version_id: null });
    }
    for (const c of home.changesets) {
      if (path === `${API}/changesets/${c.id}`) return json(200, { changeset: c });
      if (path === `${API}/changesets/${c.id}/review`) {
        return json(200, { changeset: c, docs: seed.reviews.get(c.id) ?? [], decisions: [] });
      }
      if (path === `${API}/changesets/${c.id}/conflicts`) return json(200, { conflicts: [] });
    }
    const t = home.threads.find((candidate) => path === `${API}/threads/${candidate.id}`);
    if (t) return json(200, { thread: { ...t, posts: [] } });
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

/** The sidebar's navigation. */
async function sidebar() {
  return screen.findByRole("navigation", { name: /workspace/i });
}

/** The page itself, without the sidebar. */
async function page() {
  return screen.findByRole("main");
}

/** The links of `container` that go to `href`. */
function linksTo(container: HTMLElement, href: string) {
  return Array.from(container.querySelectorAll("a")).filter((a) => a.getAttribute("href") === href);
}

/** The deepest elements of `container` whose text matches `pattern`, however the text is split into elements. */
function textElements(container: HTMLElement, pattern: RegExp) {
  return Array.from(container.querySelectorAll("*")).filter(
    (element) =>
      pattern.test(element.textContent ?? "") &&
      !Array.from(element.children).some((child) => pattern.test(child.textContent ?? "")),
  );
}

/** The entry of the review's Docs section that shows the doc `title`. */
async function reviewedEntry(main: HTMLElement, title: string) {
  const docs = await waitFor(() => {
    const found = within(main).getByRole("region", { name: /^(docs|changes|changed docs)\b/i });
    return found;
  });
  const shown = await within(docs).findByText(title);
  const entry = shown.closest('li, article, [role="listitem"]');
  expect(entry, `"${title}" is in a list item or an article`).toBeTruthy();
  return entry as HTMLElement;
}

/** Whether `container` holds a badge that reads "Global". */
function hasGlobalBadge(container: HTMLElement) {
  return within(container).queryAllByText(/^\s*global\s*$/i).length > 0;
}

/** The text of the feature's entry in the sidebar, without its name. */
function besideName(link: HTMLElement, name: string) {
  const entry = (link.closest("li") as HTMLElement | null) ?? link;
  return (entry.textContent ?? "").replace(name, "");
}

type Crumb = { name: RegExp; href: string };

/**
 * Asserts that the breadcrumb shows `crumbs` in order, each a link to its
 * href, then `section`, which goes to `sectionHref` if it is a link.
 */
async function expectTrail(crumbs: Crumb[], section: RegExp, sectionHref: string) {
  const trail = await screen.findByRole("navigation", { name: /breadcrumb/i });
  await waitFor(() => {
    const text = trail.textContent ?? "";
    let from = 0;
    for (const name of [...crumbs.map((crumb) => crumb.name), section]) {
      const match = name.exec(text.slice(from));
      expect(match, `${name} in the breadcrumb, after what comes before it (${text})`).toBeTruthy();
      from += (match?.index ?? 0) + (match?.[0].length ?? 0);
    }
  });
  for (const crumb of crumbs) {
    const links = Array.from(trail.querySelectorAll("a")).filter((a) => crumb.name.test(a.textContent ?? ""));
    expect(
      links.map((a) => a.getAttribute("href")),
      `the breadcrumb links ${crumb.name} to ${crumb.href}`,
    ).toContain(crumb.href);
  }
  for (const element of textElements(trail, section)) {
    const link = element.closest("a");
    if (link) expect(link.getAttribute("href"), `the breadcrumb's ${section}`).toBe(sectionHref);
  }
  return trail;
}

const PROJECT_CRUMB = { name: new RegExp(PROJECT_NAME), href: PROJECT_PATH };
const WISHLIST_CRUMB = { name: /wishlist/i, href: featurePath(WISHLIST) };
const MARK_DONE = /^\s*mark (as |it )?done\b/i;
const REOPEN = /^\s*reopen\b/i;

describe("feature scope in reviews", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    vi.stubGlobal("WebSocket", FakeWebSocket);
    // A confirmation is asked in the page; a browser dialog would answer no.
    vi.stubGlobal("confirm", () => false);
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  describe("the change set review", () => {
    test("a feature's change set links its feature by name", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/changesets/${seed.changesets.shareSaved.id}`);
      const main = await page();
      await within(main).findByRole("heading", { level: 1, name: "Share saved lists" });
      await waitFor(() => {
        const links = linksTo(main, featurePath(WISHLIST));
        expect(links.length, "a link to the Wishlist's overview").toBeGreaterThan(0);
        expect(links.map((link) => link.textContent).join(" ")).toMatch(/wishlist/i);
      });
    });

    test("a project-level change set names the project and links no feature", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/changesets/${seed.changesets.hero.id}`);
      const main = await page();
      await within(main).findByRole("heading", { level: 1, name: "Rework the hero" });
      await waitFor(() => expect(main.textContent, "the page names the project").toContain(PROJECT_NAME));
      for (const link of Array.from(main.querySelectorAll("a"))) {
        expect(link.getAttribute("href"), "no link to a feature").not.toMatch(/\/features\//);
      }
      expect(main.textContent).not.toMatch(/wishlist|loyalty points/i);
    });

    test("each global doc that a feature's change set changes has a Global badge, its own doc none", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/changesets/${seed.changesets.shareSaved.id}`);
      const main = await page();
      const shop = await reviewedEntry(main, "Shop PRD");
      const system = await reviewedEntry(main, "Design system");
      const saved = await reviewedEntry(main, "Saved items PRD");
      await waitFor(() => expect(hasGlobalBadge(shop), "the shop PRD is global").toBe(true));
      expect(hasGlobalBadge(system), "the design system is global").toBe(true);
      expect(hasGlobalBadge(saved), "the Wishlist's own PRD is not global").toBe(false);
    });

    test("the merge box says how many global docs the change set changes, ahead of Approve", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/changesets/${seed.changesets.shareSaved.id}`);
      const main = await page();
      const approve = await within(main).findByRole("button", { name: /^approve/i });
      const statement = await waitFor(() => {
        const found = textElements(main, /changes\s+2\s+global\s+docs/i);
        expect(found.length, 'the merge box says "Changes 2 global docs"').toBeGreaterThan(0);
        return found[0];
      });
      expect(
        statement.compareDocumentPosition(approve) & Node.DOCUMENT_POSITION_FOLLOWING,
        "the statement comes before Approve",
      ).toBeTruthy();
    });
  });

  describe("where an item lives", () => {
    test("a feature's thread: project › feature › Threads", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/threads/${seed.threads.savedThread.id}`);
      await within(await page()).findByRole("heading", { level: 1, name: "Should saved lists be public?" });
      await expectTrail([PROJECT_CRUMB, WISHLIST_CRUMB], /threads/i, featurePath(WISHLIST, "threads"));
    });

    test("a project-level thread: project › Threads, no feature", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/threads/${seed.threads.heroThread.id}`);
      await within(await page()).findByRole("heading", { level: 1, name: "Which hero layout?" });
      const trail = await expectTrail([PROJECT_CRUMB], /threads/i, `${PROJECT_PATH}/threads`);
      expect(trail.textContent).not.toMatch(/wishlist|loyalty points/i);
      for (const link of Array.from(trail.querySelectorAll("a"))) {
        expect(link.getAttribute("href"), "no crumb goes to a feature").not.toMatch(/\/features\//);
      }
    });

    test("a feature's doc in the doc viewer: project › feature › Docs", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/docs/${seed.docs.savedPrd.id}`);
      // The doc viewer replaces its loading page once the doc is there.
      await screen.findByRole("heading", { level: 1, name: "Saved items PRD" });
      await expectTrail([PROJECT_CRUMB, WISHLIST_CRUMB], /docs/i, featurePath(WISHLIST, "docs"));
    });

    test("a feature's design in the design viewer: project › feature › Docs", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/docs/${seed.docs.savedScreens.id}`);
      // The doc viewer replaces its loading page once the doc is there.
      await screen.findByRole("heading", { level: 1, name: "Saved items screens" });
      await expectTrail([PROJECT_CRUMB, WISHLIST_CRUMB], /docs/i, featurePath(WISHLIST, "docs"));
    });
  });

  describe("a feature has no status", () => {
    test.each([
      ["Wishlist", WISHLIST, /wishlist/i],
      ["Loyalty points", LOYALTY, /loyalty points/i],
    ])("the overview of %s has no Mark done and no Reopen button and sends nothing", async (_, id, name) => {
      const api = fakeApi(seeded());
      renderAt(featurePath(id));
      const main = await page();
      await within(main).findByRole("heading", { level: 1, name });
      await waitFor(() => expect(main.textContent, "the overview has loaded its sections").toMatch(/threads/i));
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 50));
      });
      expect(screen.queryByRole("button", { name: MARK_DONE }), "no Mark done").toBeNull();
      expect(screen.queryByRole("button", { name: REOPEN }), "no Reopen").toBeNull();
      expect(api.calls, "nothing is sent").toEqual([]);
    });

    test.each([
      ["overview", ""],
      ["Docs", "docs"],
      ["Change sets", "changesets"],
      ["Threads", "threads"],
    ])("no feature's %s page shows a Done badge", async (name, section) => {
      for (const [id, featureName] of [
        [WISHLIST, /wishlist/i],
        [LOYALTY, /loyalty points/i],
      ] as const) {
        fakeApi(seeded());
        renderAt(featurePath(id, section));
        const main = await page();
        await waitFor(() => expect(main.textContent, `the ${name} page names the feature`).toMatch(featureName));
        expect(within(main).queryAllByText(/^\s*done\s*$/i), `no Done badge on the ${name} page`).toEqual([]);
        cleanup();
      }
    });

    test("the sidebar lists every feature, with no Done toggle and no Done badge", async () => {
      fakeApi(seeded());
      renderAt(PROJECT_PATH);
      const nav = await sidebar();
      const wishlist = await waitFor(() => {
        const found = linksTo(nav, featurePath(WISHLIST));
        expect(found.length).toBeGreaterThan(0);
        return found[0];
      });
      const loyalty = await waitFor(() => {
        const found = linksTo(nav, featurePath(LOYALTY));
        expect(found.length, "Loyalty points is listed with the others").toBeGreaterThan(0);
        return found[0];
      });
      expect(within(nav).queryByRole("button", { name: /^done\b/i }), "no Done toggle").toBeNull();
      expect(besideName(loyalty, "Loyalty points"), "the Loyalty points' entry").not.toMatch(/\bdone\b/i);
      expect(besideName(wishlist, "Wishlist"), "the Wishlist's entry").not.toMatch(/\bdone\b/i);
    });
  });
});
