// @vitest-environment jsdom
// US-064: features in the web app. A project holds global docs, change sets
// and threads, and features, each with its own (FR-52..FR-55). The app is
// rendered with its own router on a memory history, the browser's fetch is
// replaced by a fake of the API, and the browser's WebSocket by a fake socket.
//
// The pages read the project's home, GET .../projects/<project>/home
// (crates/omni-api/tests/project_home.rs). Its docs, change sets, threads and
// waiting items each carry `feature_id` (null for the project level), and
// `features` lists each feature as {feature, docs, open_changesets, waiting}.
// Since US-110 a feature has no status.
//
// The sidebar (the navigation named "Workspace") has, below the project's
// sections (Overview, Inbox, Docs, Change sets, Threads), a group named
// "Features" (role group or region, named by its heading) that links every
// feature to its overview, /w/<workspace>/projects/<project>/features/
// <feature>, with its waiting count next to its name. There is no "Done"
// toggle (US-110).
//
// A feature has an overview page (its name as the page's heading, its
// description, and sections named "Waiting for you", "Docs", "Change sets" and
// "Threads") and .../features/<feature>/docs, /changesets and /threads pages,
// each holding that feature's items only. The project's Docs, Change sets and
// Threads pages hold the project-level items only; its overview and inbox hold
// everything, each feature's items tagged with the feature's name.
//
// The sidebar marks where the reviewer is with aria-current: on a feature's
// page, the link to that feature's section (its overview, docs, change sets
// or threads); on a doc, change set or thread of a feature, the link to that
// feature's docs, change sets or threads, and that page's back link goes there
// too (for a doc, the viewer toolbar's "Back to the docs" link).
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-22T10:00:00Z";
const AGENT = { kind: "agent", run: "run-1", role: "critic", model: "test-model", harness: "test-harness" };
const HUMAN = { kind: "human", user_id: USER.id };

const WISHLIST = "0192d7a0-7777-7000-8000-000000000001";
const GIFT_CARDS = "0192d7a0-7777-7000-8000-000000000002";
const LOYALTY = "0192d7a0-7777-7000-8000-000000000003";
const FEATURE_NAMES = /wishlist|gift cards|loyalty points/i;

let counter = 0;
/** A UUIDv7-shaped ID that no other fixture has. */
function newId() {
  counter += 1;
  return `0192d7a0-8888-7000-8000-${counter.toString(16).padStart(12, "0")}`;
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

function doc(featureId: string | null, pillar: string, title: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: featureId,
    kind: pillar === "ui" ? "design" : "html",
    pillar,
    title,
    created_at: AT,
    mainline_version_id: null,
    drafts: [],
    claims: [],
  };
}

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

function thread(changeset: { id: string; feature_id: string | null }, question: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: changeset.feature_id,
    changeset_id: changeset.id,
    question,
    criteria: ["Readable at 320 px"],
    run_tag: "run-1",
    created_at: AT,
    phases: [{ number: 1, sealed: false, started_at: AT, posted_by: [] }],
    decision: null,
  };
}

function decision(on: ReturnType<typeof thread>, direction: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: on.feature_id,
    thread_id: on.id,
    changeset_id: on.changeset_id,
    status: "proposed",
    author: AGENT,
    direction,
    chosen: "Option A",
    criteria: ["Readable at 320 px"],
    options: [{ name: "Option A", scores: { "Readable at 320 px": 8 }, lost_because: null }],
    assumptions: [],
    open_questions: [],
    dissent: [],
    posts: [],
    rejection_reason: null,
    created_at: AT,
  };
}

function comment(on: ReturnType<typeof doc>, changesetId: string, text: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    feature_id: on.feature_id,
    doc_id: on.id,
    version_id: newId(),
    node: "abcdefgh",
    changeset_id: changesetId,
    author: HUMAN,
    text,
    status: "addressed",
    addressed_in: newId(),
    replies: [],
    node_status: "current",
    compared_to: null,
    created_at: AT,
  };
}

function question(on: ReturnType<typeof thread>, summary: string) {
  return {
    id: newId(),
    feature_id: on.feature_id,
    thread_id: on.id,
    phase: 1,
    type: "question",
    author: AGENT,
    summary,
    body: "Tell me more.",
    body_html: "<p>Tell me more.</p>",
    links: [],
    attachments: [],
    confidence: null,
    replaces: null,
    created_at: AT,
  };
}

/**
 * A project with project-level items, two active features (Wishlist and Gift
 * cards) and one with nothing open (Loyalty points, whose legacy status says
 * done). No item's title holds a feature's name, so a feature's name next to
 * an item is its tag.
 */
function seeded() {
  const wishlist = feature(WISHLIST, "Wishlist", "Shoppers save products for later.", "active");
  const giftCards = feature(GIFT_CARDS, "Gift cards", "Buy and redeem vouchers.", "active");
  const loyalty = feature(LOYALTY, "Loyalty points", "Earn points on every order.", "done");

  const storefrontPrd = doc(null, "spec", "Storefront PRD");
  const ordersSchema = doc(null, "db", "Orders schema");
  const savedPrd = doc(WISHLIST, "spec", "Saved items PRD");
  const savedScreens = doc(WISHLIST, "ui", "Saved items screens");
  const voucherPrd = doc(GIFT_CARDS, "spec", "Voucher PRD");
  const pointsPrd = doc(LOYALTY, "spec", "Points PRD");

  const footer = changeset(null, "Tidy the footer", "open");
  const hero = changeset(null, "Rework the hero", "in_review");
  const draftSaved = changeset(WISHLIST, "Draft saved items", "open");
  const shareSaved = changeset(WISHLIST, "Share saved lists", "in_review");
  const voucherBalance = changeset(GIFT_CARDS, "Voucher balance", "open");
  const ledger = changeset(LOYALTY, "Points ledger", "merged");

  const heroThread = thread(hero, "Which hero layout?");
  const savedThread = thread(draftSaved, "Should saved lists be public?");
  const voucherThread = thread(voucherBalance, "How do vouchers expire?");

  const home = {
    project_id: PROJECT_ID,
    docs: [storefrontPrd, ordersSchema, savedPrd, savedScreens, voucherPrd, pointsPrd],
    changesets: [footer, hero, draftSaved, shareSaved, voucherBalance, ledger],
    threads: [heroThread, savedThread, voucherThread],
    waiting: {
      decisions: [decision(heroThread, "Use the split hero"), decision(savedThread, "Keep saved lists private")],
      changesets: [hero, shareSaved],
      comments: [
        comment(storefrontPrd, hero.id, "The hero copy is shorter now"),
        comment(savedPrd, draftSaved.id, "The save button moved"),
      ],
      questions: [question(voucherThread, "Do vouchers stack?")],
    },
    features: [
      { feature: wishlist, docs: 2, open_changesets: 2, waiting: 3 },
      { feature: giftCards, docs: 1, open_changesets: 1, waiting: 1 },
      { feature: loyalty, docs: 1, open_changesets: 0, waiting: 0 },
    ],
  };
  return {
    home,
    features: [wishlist, giftCards, loyalty],
    docs: { storefrontPrd, savedPrd },
    changesets: { hero, shareSaved },
    threads: { savedThread },
  };
}

type Seeded = ReturnType<typeof seeded>;

/** A fake of the API: the auth routes, the project, its home, its features and the reads of its items. */
function fakeApi(seed: Seeded) {
  const { home } = seed;
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url, "http://localhost:3000").pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    switch (`${method} ${path}`) {
      case "GET /api/v1/auth/me":
        return json(200, { user: USER });
      case "GET /api/v1/auth/workspaces":
        return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
      case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
        return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
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
    if (method !== "GET") return json(404, { error: "not_found" });
    const feature = seed.features.find((f) => path === `${API}/features/${f.id}`);
    if (feature) return json(200, { feature });
    for (const d of home.docs) {
      if (path === `${API}/docs/${d.id}`) return json(200, { doc: d });
      if (path === `${API}/docs/${d.id}/versions`) return json(200, { doc_id: d.id, versions: [] });
    }
    for (const c of home.changesets) {
      if (path === `${API}/changesets/${c.id}`) return json(200, { changeset: c });
      if (path === `${API}/changesets/${c.id}/review`) return json(200, { changeset: c, docs: [], decisions: [] });
      if (path === `${API}/changesets/${c.id}/conflicts`) return json(200, { conflicts: [] });
    }
    const t = home.threads.find((candidate) => path === `${API}/threads/${candidate.id}`);
    if (t) return json(200, { thread: { ...t, posts: [] } });
    return json(404, { error: "not_found" });
  });
  vi.stubGlobal("fetch", fetch);
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

/** The one link of `container` to `href`, once it is there. */
async function linkTo(container: HTMLElement, href: string) {
  return waitFor(() => {
    const found = linksTo(container, href);
    expect(found.length, `a link to ${href}`).toBeGreaterThan(0);
    return found[0];
  });
}

/** Whether `element` is marked as where the reviewer is. */
function marked(element: Element) {
  const current = element.getAttribute("aria-current");
  return current !== null && current !== "false";
}

/** The links of `container` that are marked as where the reviewer is. */
function markedLinks(container: HTMLElement) {
  return Array.from(container.querySelectorAll("a")).filter(marked);
}

/** The section (or group) named `name` inside `container`. */
async function section(name: RegExp, container: HTMLElement) {
  return waitFor(() => {
    const found =
      within(container).queryByRole("region", { name }) ?? within(container).queryByRole("group", { name });
    expect(found, `a section named ${name}`).toBeTruthy();
    return found as HTMLElement;
  });
}

/** The list item or table row of `container` that shows `text`. */
function itemShowing(container: HTMLElement, text: string) {
  const element = within(container).getByText(text);
  const item = element.closest('li, tr, [role="listitem"], [role="row"]');
  expect(item, `"${text}" is in a list item or a table row`).toBeTruthy();
  return item as HTMLElement;
}

/** The text of the feature's entry in the sidebar, without its name. */
function besideName(link: HTMLElement, name: string) {
  const entry = (link.closest("li") as HTMLElement | null) ?? link;
  return (entry.textContent ?? "").replace(name, "");
}

/** Asserts that `container` shows every one of `shown` and none of `hidden`. */
async function showsOnly(container: HTMLElement, shown: string[], hidden: string[]) {
  await within(container).findByText(shown[0]);
  for (const text of shown) expect(within(container).queryByText(text), `${text} is shown`).toBeTruthy();
  for (const text of hidden) expect(within(container).queryByText(text), `${text} is not shown`).toBeNull();
}

const WISHLIST_DOCS = ["Saved items PRD", "Saved items screens"];
const OTHER_DOCS = ["Storefront PRD", "Orders schema", "Voucher PRD", "Points PRD"];
const WISHLIST_CHANGESETS = ["Draft saved items", "Share saved lists"];
const OTHER_CHANGESETS = ["Tidy the footer", "Rework the hero", "Voucher balance", "Points ledger"];
const WISHLIST_THREADS = ["Should saved lists be public?"];
const OTHER_THREADS = ["Which hero layout?", "How do vouchers expire?"];

describe("features in the web app", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  describe("the sidebar", () => {
    test("below the project's sections, a Features group lists every feature with its waiting count", async () => {
      fakeApi(seeded());
      renderAt(PROJECT_PATH);
      const nav = await sidebar();
      const threads = await linkTo(nav, `${PROJECT_PATH}/threads`);
      const features = await section(/^features\b/i, nav);
      expect(
        threads.compareDocumentPosition(features) & Node.DOCUMENT_POSITION_FOLLOWING,
        "the Features group comes after the project's sections",
      ).toBeTruthy();

      const wishlist = await linkTo(features, featurePath(WISHLIST));
      expect(wishlist.textContent).toMatch(/wishlist/i);
      expect(besideName(wishlist, "Wishlist"), "the Wishlist's waiting count").toMatch(/(^|\D)3(\D|$)/);
      const giftCards = await linkTo(features, featurePath(GIFT_CARDS));
      expect(giftCards.textContent).toMatch(/gift cards/i);
      expect(besideName(giftCards, "Gift cards"), "the Gift cards' waiting count").toMatch(/(^|\D)1(\D|$)/);

      const loyalty = await linkTo(features, featurePath(LOYALTY));
      expect(loyalty.textContent).toMatch(/loyalty points/i);
      expect(within(nav).queryByRole("button", { name: /^done\b/i }), "there is no Done toggle").toBeNull();
    });
  });

  describe("the project's pages", () => {
    test("the project's Docs page shows project-level docs only", async () => {
      fakeApi(seeded());
      renderAt(`${PROJECT_PATH}/docs`);
      await showsOnly(
        await page(),
        ["Storefront PRD", "Orders schema"],
        [...WISHLIST_DOCS, "Voucher PRD", "Points PRD"],
      );
    });

    test("the Designs tab of the Docs page holds the design system, so the tabs add up to All", async () => {
      const seed = seeded();
      const system = { ...doc(null, "ui", "Design system"), kind: "design-system" };
      seed.home.docs.push(system);
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/docs`);
      const main = await page();
      const count = (name: RegExp) => {
        const tab = within(main).getByRole("tab", { name });
        return Number(tab.textContent?.match(/\d+/)?.[0]);
      };
      await within(main).findByRole("tab", { name: /^designs/i });
      expect(count(/^all/i), "All counts the three project docs").toBe(3);
      expect(count(/^docs/i) + count(/^designs/i), "Docs and Designs add up to All").toBe(3);

      fireEvent.click(within(main).getByRole("tab", { name: /^designs/i }));
      await within(main).findByText("Design system");
      expect(within(main).queryByText("Storefront PRD"), "written docs are not under Designs").toBeNull();
    });

    test("the project's Change sets page shows project-level change sets only", async () => {
      fakeApi(seeded());
      renderAt(`${PROJECT_PATH}/changesets`);
      await showsOnly(
        await page(),
        ["Tidy the footer", "Rework the hero"],
        [...WISHLIST_CHANGESETS, "Voucher balance", "Points ledger"],
      );
    });

    test("the project's Threads page shows project-level threads only", async () => {
      fakeApi(seeded());
      renderAt(`${PROJECT_PATH}/threads`);
      await showsOnly(await page(), ["Which hero layout?"], [...WISHLIST_THREADS, "How do vouchers expire?"]);
    });

    test("the overview shows every item, each feature's tagged with the feature's name", async () => {
      fakeApi(seeded());
      renderAt(PROJECT_PATH);
      const main = await page();

      const docs = await section(/^docs\b/i, main);
      await within(docs).findByText("Saved items PRD");
      expect(itemShowing(docs, "Saved items PRD").textContent).toMatch(/wishlist/i);
      expect(itemShowing(docs, "Voucher PRD").textContent).toMatch(/gift cards/i);
      expect(itemShowing(docs, "Points PRD").textContent).toMatch(/loyalty points/i);
      expect(itemShowing(docs, "Storefront PRD").textContent).not.toMatch(FEATURE_NAMES);

      const changesets = await section(/^change ?sets\b/i, main);
      expect(itemShowing(changesets, "Draft saved items").textContent).toMatch(/wishlist/i);
      expect(itemShowing(changesets, "Voucher balance").textContent).toMatch(/gift cards/i);
      expect(itemShowing(changesets, "Tidy the footer").textContent).not.toMatch(FEATURE_NAMES);

      const threads = await section(/^threads\b/i, main);
      expect(itemShowing(threads, "Should saved lists be public?").textContent).toMatch(/wishlist/i);
      expect(itemShowing(threads, "How do vouchers expire?").textContent).toMatch(/gift cards/i);
      expect(itemShowing(threads, "Which hero layout?").textContent).not.toMatch(FEATURE_NAMES);

      const waiting = await section(/^waiting for you\b/i, main);
      expect(itemShowing(waiting, "Keep saved lists private").textContent).toMatch(/wishlist/i);
      expect(itemShowing(waiting, "Share saved lists").textContent).toMatch(/wishlist/i);
      expect(itemShowing(waiting, "The save button moved").textContent).toMatch(/wishlist/i);
      expect(itemShowing(waiting, "Do vouchers stack?").textContent).toMatch(/gift cards/i);
      expect(itemShowing(waiting, "Use the split hero").textContent).not.toMatch(FEATURE_NAMES);
      expect(itemShowing(waiting, "Rework the hero").textContent).not.toMatch(FEATURE_NAMES);
      expect(itemShowing(waiting, "The hero copy is shorter now").textContent).not.toMatch(FEATURE_NAMES);
    });

    test("the inbox shows everything that waits, each feature's tagged with the feature's name", async () => {
      fakeApi(seeded());
      renderAt(`${PROJECT_PATH}/inbox`);
      const main = await page();
      await within(main).findByText("Keep saved lists private");
      expect(itemShowing(main, "Keep saved lists private").textContent).toMatch(/wishlist/i);
      expect(itemShowing(main, "Share saved lists").textContent).toMatch(/wishlist/i);
      expect(itemShowing(main, "The save button moved").textContent).toMatch(/wishlist/i);
      expect(itemShowing(main, "Do vouchers stack?").textContent).toMatch(/gift cards/i);
      expect(itemShowing(main, "Use the split hero").textContent).not.toMatch(FEATURE_NAMES);
      expect(itemShowing(main, "Rework the hero").textContent).not.toMatch(FEATURE_NAMES);
      expect(itemShowing(main, "The hero copy is shorter now").textContent).not.toMatch(FEATURE_NAMES);
    });
  });

  describe("a feature's pages", () => {
    test("the feature's overview shows its description, what waits, its docs, change sets and threads", async () => {
      fakeApi(seeded());
      renderAt(featurePath(WISHLIST));
      const main = await page();
      expect(await within(main).findByRole("heading", { level: 1, name: /wishlist/i })).toBeTruthy();
      expect(within(main).getByText("Shoppers save products for later.")).toBeTruthy();

      const waiting = await section(/^waiting for you\b/i, main);
      await showsOnly(
        waiting,
        ["Keep saved lists private", "Share saved lists", "The save button moved"],
        ["Use the split hero", "Rework the hero", "The hero copy is shorter now", "Do vouchers stack?"],
      );
      await showsOnly(await section(/^docs\b/i, main), WISHLIST_DOCS, OTHER_DOCS);
      await showsOnly(await section(/^change ?sets\b/i, main), WISHLIST_CHANGESETS, OTHER_CHANGESETS);
      await showsOnly(await section(/^threads\b/i, main), WISHLIST_THREADS, OTHER_THREADS);
    });

    test("the feature's Docs page shows its own docs only", async () => {
      fakeApi(seeded());
      renderAt(featurePath(WISHLIST, "docs"));
      await showsOnly(await page(), WISHLIST_DOCS, OTHER_DOCS);
    });

    test("the feature's Change sets page shows its own change sets only", async () => {
      fakeApi(seeded());
      renderAt(featurePath(WISHLIST, "changesets"));
      await showsOnly(await page(), WISHLIST_CHANGESETS, OTHER_CHANGESETS);
    });

    test("the feature's Threads page shows its own threads only", async () => {
      fakeApi(seeded());
      renderAt(featurePath(WISHLIST, "threads"));
      await showsOnly(await page(), WISHLIST_THREADS, OTHER_THREADS);
    });
  });

  describe("marking where the reviewer is", () => {
    test("on a feature's page the sidebar marks that feature's section, not the project's", async () => {
      fakeApi(seeded());
      renderAt(featurePath(WISHLIST, "changesets"));
      const nav = await sidebar();
      const current = await linkTo(nav, featurePath(WISHLIST, "changesets"));
      await waitFor(() => expect(marked(current), "the Wishlist's change sets are marked").toBe(true));
      for (const link of markedLinks(nav)) {
        expect(link.getAttribute("href"), "only the Wishlist is marked").toMatch(
          new RegExp(`^${featurePath(WISHLIST)}(/|$)`),
        );
      }
      expect(marked(await linkTo(nav, `${PROJECT_PATH}/changesets`)), "the project's change sets").toBe(false);
    });

    test("on a feature's overview the sidebar marks the feature, not the project's overview", async () => {
      fakeApi(seeded());
      renderAt(featurePath(WISHLIST));
      const nav = await sidebar();
      await waitFor(() =>
        expect(
          linksTo(nav, featurePath(WISHLIST)).some(marked),
          "the Wishlist's overview is marked",
        ).toBe(true),
      );
      expect(marked(await linkTo(nav, PROJECT_PATH)), "the project's overview").toBe(false);
      expect(linksTo(nav, featurePath(GIFT_CARDS)).some(marked), "another feature").toBe(false);
    });

    test("on a feature's change set the sidebar marks the feature's change sets and the back link goes there", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/changesets/${seed.changesets.shareSaved.id}`);
      const nav = await sidebar();
      const current = await linkTo(nav, featurePath(WISHLIST, "changesets"));
      await waitFor(() => expect(marked(current), "the Wishlist's change sets are marked").toBe(true));
      expect(marked(await linkTo(nav, `${PROJECT_PATH}/changesets`)), "the project's change sets").toBe(false);

      const main = await page();
      await within(main).findAllByText("Share saved lists");
      const back = within(main).getAllByRole("link", { name: /change sets/i });
      expect(back.map((link) => link.getAttribute("href"))).toContain(featurePath(WISHLIST, "changesets"));
      expect(back.map((link) => link.getAttribute("href"))).not.toContain(`${PROJECT_PATH}/changesets`);
    });

    test("on a feature's thread the sidebar marks the feature's threads and the back link goes there", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/threads/${seed.threads.savedThread.id}`);
      const nav = await sidebar();
      const current = await linkTo(nav, featurePath(WISHLIST, "threads"));
      await waitFor(() => expect(marked(current), "the Wishlist's threads are marked").toBe(true));
      expect(marked(await linkTo(nav, `${PROJECT_PATH}/threads`)), "the project's threads").toBe(false);

      const main = await page();
      await within(main).findByRole("heading", { level: 1, name: "Should saved lists be public?" });
      const back = within(main).getAllByRole("link", { name: /threads/i });
      expect(back.map((link) => link.getAttribute("href"))).toContain(featurePath(WISHLIST, "threads"));
      expect(back.map((link) => link.getAttribute("href"))).not.toContain(`${PROJECT_PATH}/threads`);
    });

    test("on a feature's doc the sidebar marks the feature's docs and the back link goes there", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/docs/${seed.docs.savedPrd.id}`);
      const nav = await sidebar();
      const current = await linkTo(nav, featurePath(WISHLIST, "docs"));
      await waitFor(() => expect(marked(current), "the Wishlist's docs are marked").toBe(true));
      expect(marked(await linkTo(nav, `${PROJECT_PATH}/docs`)), "the project's docs").toBe(false);

      const main = await page();
      await within(main).findByRole("heading", { level: 1, name: "Saved items PRD" });
      await waitFor(() => {
        const back = within(main).getAllByRole("link", { name: /docs/i });
        expect(back.map((link) => link.getAttribute("href"))).toContain(featurePath(WISHLIST, "docs"));
        expect(back.map((link) => link.getAttribute("href"))).not.toContain(`${PROJECT_PATH}/docs`);
      });
    });

    test("on a project-level doc the sidebar marks the project's docs and the back link goes there", async () => {
      const seed = seeded();
      fakeApi(seed);
      renderAt(`${PROJECT_PATH}/docs/${seed.docs.storefrontPrd.id}`);
      const nav = await sidebar();
      const current = await linkTo(nav, `${PROJECT_PATH}/docs`);
      await waitFor(() => expect(marked(current), "the project's docs are marked").toBe(true));
      await linkTo(nav, featurePath(WISHLIST));
      for (const link of markedLinks(nav)) {
        expect(link.getAttribute("href"), "no feature is marked").not.toMatch(/\/features\//);
      }

      const main = await page();
      await within(main).findByRole("heading", { level: 1, name: "Storefront PRD" });
      const back = within(main).getAllByRole("link", { name: /docs/i });
      expect(back.map((link) => link.getAttribute("href"))).toContain(`${PROJECT_PATH}/docs`);
      for (const link of back) {
        expect(link.getAttribute("href"), "no back link to a feature").not.toMatch(/\/features\//);
      }
    });
  });
});
