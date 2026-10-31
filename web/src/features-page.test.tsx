// @vitest-environment jsdom
// US-125: the Features page, .../projects/<project>/features, and the palette
// that the sidebar's Search item or ⌘K (Ctrl+K off macOS) opens. Both read the
// project home, GET .../projects/<project>/home. The app is rendered into the
// document, as in the browser: the palette's portal on the body then sees the
// same events as there.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const CHECKOUT = "0192d7a0-4444-7000-8000-000000000001";
const WALLETS = "0192d7a0-4444-7000-8000-000000000002";
const RETURNS = "0192d7a0-4444-7000-8000-000000000003";
const GIFT_CARDS = "0192d7a0-4444-7000-8000-000000000004";
const APPLE_PAY = "0192d7a0-4444-7000-8000-000000000005";
const PAYMENTS = "0192d7a0-4444-7000-8000-000000000006";
const PAY_LATER = "0192d7a0-4444-7000-8000-000000000007";
const CHECKOUT_DOC = "0192d7a0-5555-7000-8000-000000000001";
const CHECKOUT_THREAD = "0192d7a0-6666-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const VISITS_KEY = `omni:visits:${PROJECT_ID}`;
const AT = "2026-10-21T08:00:00Z";

type Stories = { total: number; open: number; done: number };

function feature(
  id: string,
  name: string,
  fields: { description?: string; waiting?: number; lastActivity?: string | null; createdAt?: string; stories?: Stories } = {},
) {
  return {
    feature: { id, project_id: PROJECT_ID, name, description: fields.description ?? "", created_at: fields.createdAt ?? AT },
    docs: 0,
    waiting: fields.waiting ?? 0,
    stories: fields.stories ?? { total: 0, open: 0, done: 0 },
    last_activity: fields.lastActivity ?? null,
  };
}

function doc(id: string, featureId: string | null, title: string) {
  return { id, project_id: PROJECT_ID, feature_id: featureId, kind: "html", pillar: "spec", waiting: 0, title, created_at: AT, drafts: [], claims: [] };
}

function thread(id: string, featureId: string | null, question: string) {
  return {
    id,
    project_id: PROJECT_ID,
    feature_id: featureId,
    question,
    sealed: false,
    conclusion: null,
    run_tag: null,
    created_at: AT,
    post_count: 0,
    last_post_at: null,
  };
}

function home(fields: { features?: ReturnType<typeof feature>[]; docs?: ReturnType<typeof doc>[]; threads?: ReturnType<typeof thread>[] }) {
  return {
    project_id: PROJECT_ID,
    app: { last_activity: null },
    features: fields.features ?? [],
    docs: fields.docs ?? [],
    threads: fields.threads ?? [],
    waiting: { comments: [] },
  };
}

type Home = ReturnType<typeof home>;

/** A fake of the API: the auth routes, the workspace's project, its home and its comments. */
function fakeApi(projectHome: Home) {
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      switch (new URL(url, "http://localhost:3000").pathname) {
        case "/api/v1/auth/me":
          return json(200, { user: USER });
        case "/api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `/api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
        case `${API}/home`:
          return json(200, projectHome);
        case `${API}/comments`:
          return json(200, { project_id: PROJECT_ID, comments: [] });
        default:
          return json(404, { error: "not_found" });
      }
    }),
  );
}

/** A WebSocket that never opens: these pages are not tested live here. */
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
  onopen = null;
  onmessage = null;
  onclose = null;
  onerror = null;

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
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />, { container: document });
  return router;
}

/** The text of `element` without its decoration (what is aria-hidden). */
function shownText(element: HTMLElement) {
  const copy = element.cloneNode(true) as HTMLElement;
  for (const hidden of Array.from(copy.querySelectorAll("[aria-hidden]"))) hidden.remove();
  return (copy.textContent ?? "").trim();
}

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
  localStorage.clear();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  delete (navigator as { platform?: string }).platform;
  localStorage.clear();
});

describe("the Features page", () => {
  const features = [
    feature(RETURNS, "Returns", { lastActivity: "2026-10-21T11:00:00Z" }),
    feature(WALLETS, "Wallets", { description: "Apple Pay and Google Pay.", lastActivity: null, createdAt: "2026-10-21T12:00:00Z" }),
    feature(GIFT_CARDS, "Gift cards", { lastActivity: "2026-10-21T11:00:00Z" }),
    feature(CHECKOUT, "Checkout", {
      description: "Pay in one step.\nWorks with saved wallets.",
      waiting: 2,
      lastActivity: "2026-10-21T13:00:00Z",
      stories: { total: 3, open: 2, done: 1 },
    }),
    feature(APPLE_PAY, "apple pay", { lastActivity: "2026-10-21T11:00:00Z" }),
  ];

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-21T15:00:00Z"));
  });

  const main = () => screen.getByRole("main");
  const rows = () => within(within(main()).getByRole("list")).getAllByRole("link");
  const rowNames = () => rows().map((row) => row.querySelector("span span")?.textContent);
  const filter = (query: string) => fireEvent.change(within(main()).getByRole("textbox", { name: "Filter features" }), { target: { value: query } });

  test("lists every feature, the most recent activity first, one without activity at its creation, ties by name", async () => {
    fakeApi(home({ features }));
    renderAt(`${PROJECT_PATH}/features`);
    await waitFor(() => expect(rows()).toHaveLength(5));
    expect(rowNames()).toEqual(["Checkout", "Wallets", "apple pay", "Gift cards", "Returns"]);
    expect(within(main()).getByText("5 features")).toBeTruthy();
  });

  test("a row shows the name, the first line of the description, done of total stories, what waits and the last activity, and opens the feature", async () => {
    fakeApi(home({ features }));
    renderAt(`${PROJECT_PATH}/features`);
    await waitFor(() => expect(rows()).toHaveLength(5));
    expect(shownText(rows()[0])).toBe("CheckoutPay in one step.1/3 stories22 h ago");
    expect(rows()[0].getAttribute("href")).toBe(`${PROJECT_PATH}/features/${CHECKOUT}`);
    expect(shownText(rows()[1])).toBe("WalletsApple Pay and Google Pay.0/0 storiesNo activity yet");
  });

  test("the filter narrows the list as you type to the names and full descriptions holding the trimmed query, in the same order, and says N of M", async () => {
    fakeApi(home({ features }));
    renderAt(`${PROJECT_PATH}/features`);
    await waitFor(() => expect(rows()).toHaveLength(5));

    filter("  WALLET ");
    expect(rowNames()).toEqual(["Checkout", "Wallets"]);
    expect(within(main()).getByText("2 of 5")).toBeTruthy();

    filter("  WALLETS a");
    expect(within(main()).queryAllByRole("link")).toEqual([]);
    expect(within(main()).getByText("0 of 5")).toBeTruthy();

    filter("");
    expect(rows()).toHaveLength(5);
    expect(within(main()).getByText("5 features")).toBeTruthy();
  });
});

describe("the palette", () => {
  const payHome = home({
    features: [feature(PAYMENTS, "Payments"), feature(PAY_LATER, "Pay later"), feature(CHECKOUT, "Checkout")],
    docs: [1, 2, 3, 4, 5, 6, 7].map((n) => doc(`0192d7a0-5555-7000-8000-00000000002${n}`, null, `Pay note ${n}`)),
    threads: [thread(CHECKOUT_THREAD, CHECKOUT, "Which wallets?")],
  });

  /** Renders the Features page on `platform`, and waits until the sidebar has the project home. */
  async function renderOn(platform: string, projectHome: Home) {
    Object.defineProperty(navigator, "platform", { value: platform, configurable: true });
    fakeApi(projectHome);
    const router = renderAt(`${PROJECT_PATH}/features`);
    const nav = await screen.findByRole("navigation", { name: "Project" });
    await within(nav).findByRole("link", { name: /^Features\d/ });
    return router;
  }

  const palette = () => screen.queryByRole("dialog", { name: "Search" });
  const press = (key: string, modifiers: { metaKey?: boolean; ctrlKey?: boolean }) => fireEvent.keyDown(document.body, { key, ...modifiers });
  const input = () => within(palette()!).getByRole("combobox");
  const type = (text: string) => fireEvent.change(input(), { target: { value: text } });
  const options = () => within(palette()!).queryAllByRole("option");
  const selected = () => options().filter((option) => option.getAttribute("aria-selected") === "true").map(shownText);

  test("on a Mac, ⌘K opens it and Ctrl+K does not, and Esc closes it", async () => {
    await renderOn("MacIntel", payHome);
    press("k", { ctrlKey: true });
    expect(palette()).toBeNull();

    press("k", { metaKey: true });
    expect(palette()).toBeTruthy();
    fireEvent.keyDown(input(), { key: "Escape" });
    expect(palette()).toBeNull();
  });

  test("off macOS, Ctrl+K opens it and ⌘K does not", async () => {
    await renderOn("Linux x86_64", payHome);
    press("k", { metaKey: true });
    expect(palette()).toBeNull();

    press("k", { ctrlKey: true });
    expect(palette()).toBeTruthy();
  });

  test("the Search item opens it on the places visited last, a doc with its scope and pillar, a thread with its scope", async () => {
    const visits = [{ scope: CHECKOUT, doc: CHECKOUT_DOC }, { scope: CHECKOUT, thread: CHECKOUT_THREAD }, { scope: "app" }];
    localStorage.setItem(VISITS_KEY, JSON.stringify(visits.map((visit, n) => ({ ...visit, at: Date.parse(AT) - n * 60_000 }))));
    await renderOn("MacIntel", home({ features: [feature(CHECKOUT, "Checkout")], docs: [doc(CHECKOUT_DOC, CHECKOUT, "Pay in one step")], threads: [thread(CHECKOUT_THREAD, CHECKOUT, "Which wallets?")] }));

    fireEvent.click(within(screen.getByRole("navigation", { name: "Project" })).getByRole("button", { name: /^Search/ }));

    expect(options().map(shownText)).toEqual(["Pay in one stepCheckout · Spec", "Which wallets?Checkout", "App"]);
  });

  test("typing groups the matches as Features and Docs, says how many a full group shows of how many, and leaves out an empty group", async () => {
    await renderOn("MacIntel", payHome);
    press("k", { metaKey: true });
    type("pay");

    const groups = within(palette()!).getAllByRole("group");
    expect(groups.map((group) => group.getAttribute("aria-label"))).toEqual(["Features", "Docs"]);
    expect(within(groups[0]).queryByText(/ of /)).toBeNull();
    expect(within(groups[1]).getByText("6 of 7")).toBeTruthy();
    expect(options().map(shownText)).toEqual([
      "Pay later",
      "Payments",
      "Pay note 1App · Spec",
      "Pay note 2App · Spec",
      "Pay note 3App · Spec",
      "Pay note 4App · Spec",
      "Pay note 5App · Spec",
      "Pay note 6App · Spec",
    ]);
  });

  test("a query that only a fuzzy match would find shows no group and says the palette searches names and titles", async () => {
    await renderOn("MacIntel", payHome);
    press("k", { metaKey: true });
    type("chkout");

    expect(within(palette()!).queryAllByRole("group")).toEqual([]);
    expect(options()).toEqual([]);
    expect(within(palette()!).getByText(/searches names and titles/)).toBeTruthy();
  });

  test("the arrow keys move through the results and Enter opens the one chosen", async () => {
    const router = await renderOn("MacIntel", payHome);
    press("k", { metaKey: true });
    type("pay");
    expect(selected()).toEqual(["Pay later"]);

    fireEvent.keyDown(input(), { key: "ArrowDown" });
    fireEvent.keyDown(input(), { key: "ArrowDown" });
    expect(selected()).toEqual(["Pay note 1App · Spec"]);
    fireEvent.keyDown(input(), { key: "ArrowUp" });
    expect(selected()).toEqual(["Payments"]);

    fireEvent.keyDown(input(), { key: "Enter" });
    await waitFor(() => expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/features/${PAYMENTS}`));
    expect(palette()).toBeNull();
  });
});
