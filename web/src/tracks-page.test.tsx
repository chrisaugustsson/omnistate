// @vitest-environment jsdom
// US-125: the Tracks page, .../projects/<project>/tracks, and the palette
// that the top bar's Search button or ⌘K (Ctrl+K off macOS) opens. Both read the
// project home, GET .../projects/<project>/home. The app is rendered into the
// document, as in the browser: the palette's portal on the body then sees the
// same events as there.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
/** The part every track of the fixtures is on (US-305). */
const PART_ID = "0192d7a0-3333-7000-8000-000000000001";
const OTHER_PART_ID = "0192d7a0-3333-7000-8000-000000000002";
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

function track(
  id: string,
  name: string,
  fields: {
    description?: string;
    waiting?: number;
    lastActivity?: string | null;
    createdAt?: string;
    stories?: Stories;
    state?: string;
    partName?: string;
    partDeleted?: boolean;
    owner?: string;
  } = {},
) {
  return {
    track: {
      id,
      project_id: PROJECT_ID,
      part_id: PART_ID,
      state: fields.state ?? "planning",
      name,
      description: fields.description ?? "",
      created_at: fields.createdAt ?? AT,
      owner_user_id: fields.owner ?? USER.id,
    },
    docs: 0,
    waiting: fields.waiting ?? 0,
    stories: fields.stories ?? { total: 0, open: 0, done: 0 },
    last_activity: fields.lastActivity ?? null,
    part_name: fields.partName ?? "Wision",
    part_deleted: fields.partDeleted ?? false,
  };
}

function part(id: string, name: string) {
  return { id, project_id: PROJECT_ID, name, kind: "app", description: "", created_at: AT };
}

function doc(id: string, trackId: string | null, title: string) {
  return { id, project_id: PROJECT_ID, track_id: trackId, kind: "html", pillar: "spec", waiting: 0, title, created_at: AT, drafts: [], claims: [] };
}

function thread(id: string, trackId: string | null, question: string) {
  return {
    id,
    project_id: PROJECT_ID,
    track_id: trackId,
    question,
    sealed: false,
    conclusion: null,
    run_tag: null,
    created_at: AT,
    post_count: 0,
    last_post_at: null,
  };
}

function home(fields: { tracks?: ReturnType<typeof track>[]; docs?: ReturnType<typeof doc>[]; threads?: ReturnType<typeof thread>[] }) {
  return {
    project_id: PROJECT_ID,
    app: { last_activity: null },
    tracks: fields.tracks ?? [],
    docs: fields.docs ?? [],
    threads: fields.threads ?? [],
    waiting: { comments: [] },
  };
}

type Home = ReturnType<typeof home>;

/**
 * A fake of the API: the auth routes, the workspace's project, its home, its
 * parts and its comments. A POST of a track is recorded and answered 201.
 */
function fakeApi(projectHome: Home, parts: ReturnType<typeof part>[] = [part(PART_ID, "Wision")]) {
  const posted: unknown[] = [];
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = new URL(url, "http://localhost:3000").pathname;
      if ((init?.method ?? "GET").toUpperCase() === "POST" && path === `${API}/tracks`) {
        const body = JSON.parse(String(init?.body));
        posted.push(body);
        return json(201, { track: { ...track(CHECKOUT, body.name).track, part_id: body.part_id } });
      }
      switch (path) {
        case `${API}/parts`:
          return json(200, { parts });
        case "/api/v1/auth/me":
          return json(200, { user: USER });
        case `/api/v1/w/${WORKSPACE_ID}/whoami`:
          return json(200, { actor: { kind: "human", user_id: USER.id }, workspace_id: WORKSPACE_ID });
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
  return posted;
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

describe("the Tracks page", () => {
  const tracks = [
    track(RETURNS, "Returns", { lastActivity: "2026-10-21T11:00:00Z" }),
    track(WALLETS, "Wallets", {
      description: "Apple Pay and Google Pay.",
      lastActivity: null,
      createdAt: "2026-10-21T12:00:00Z",
      partName: "Old app",
      partDeleted: true,
    }),
    track(GIFT_CARDS, "Gift cards", { lastActivity: "2026-10-21T11:00:00Z" }),
    track(CHECKOUT, "Checkout", {
      description: "Pay in one step.\nWorks with saved wallets.",
      waiting: 2,
      lastActivity: "2026-10-21T13:00:00Z",
      stories: { total: 3, open: 2, done: 1 },
      state: "building",
    }),
    track(APPLE_PAY, "apple pay", { lastActivity: "2026-10-21T11:00:00Z" }),
  ];

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-21T15:00:00Z"));
  });

  const main = () => screen.getByRole("main");
  const rows = () => within(within(main()).getByRole("list")).getAllByRole("link");
  const rowNames = () => rows().map((row) => row.querySelector("span span")?.textContent);
  const filter = (query: string) => fireEvent.change(within(main()).getByRole("textbox", { name: "Filter tracks" }), { target: { value: query } });

  test("lists every track, the most recent activity first, one without activity at its creation, ties by name", async () => {
    fakeApi(home({ tracks }));
    renderAt(`${PROJECT_PATH}/tracks`);
    await waitFor(() => expect(rows()).toHaveLength(5));
    expect(rowNames()).toEqual(["Checkout", "Wallets", "apple pay", "Gift cards", "Returns"]);
    expect(within(main()).getByText("5 tracks")).toBeTruthy();
  });

  test("a row shows the name, the first line of the description, the part, the state, done of total stories, what waits and the last activity, and opens the track", async () => {
    fakeApi(home({ tracks }));
    renderAt(`${PROJECT_PATH}/tracks`);
    await waitFor(() => expect(rows()).toHaveLength(5));
    expect(shownText(rows()[0])).toBe("CheckoutPay in one step.Wisionbuilding1/3 stories done22 h ago");
    expect(rows()[0].getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${CHECKOUT}`);
    expect(shownText(rows()[1]), "a track on a deleted part stays, its part marked deleted").toBe(
      "WalletsApple Pay and Google Pay.Old app (deleted)planning0/0 stories doneNo activity yet",
    );
  });

  test("New track asks for a part, starting empty, and creates the track on the part picked", async () => {
    const posted = fakeApi(home({ tracks }), [part(PART_ID, "Wision"), part(OTHER_PART_ID, "iot-service")]);
    renderAt(`${PROJECT_PATH}/tracks`);
    await waitFor(() => expect(rows()).toHaveLength(5));
    fireEvent.click(await within(main()).findByRole("button", { name: "New track" }));

    const picker = within(main()).getByRole("combobox", { name: "Part" }) as HTMLSelectElement;
    expect(picker.value, "the picker starts empty").toBe("");
    expect(within(picker).getAllByRole("option").filter((option) => !(option as HTMLOptionElement).disabled).map((option) => option.textContent)).toEqual([
      "Wision",
      "iot-service",
    ]);
    fireEvent.change(within(main()).getByRole("textbox", { name: "Track name" }), { target: { value: "Alarms" } });
    const create = within(main()).getByRole("button", { name: "Create track" }) as HTMLButtonElement;
    expect(create.disabled, "no part, no create").toBe(true);

    fireEvent.change(picker, { target: { value: OTHER_PART_ID } });
    expect(create.disabled).toBe(false);
    fireEvent.click(create);
    await waitFor(() => expect(posted).toEqual([{ name: "Alarms", description: "", part_id: OTHER_PART_ID }]));
  });

  test("with no parts, New track is disabled and points to the project page to create one", async () => {
    fakeApi(home({ tracks }), []);
    renderAt(`${PROJECT_PATH}/tracks`);
    await waitFor(() => expect(rows()).toHaveLength(5));
    const create = (await within(main()).findByRole("button", { name: "New track" })) as HTMLButtonElement;
    expect(create.disabled).toBe(true);
    const hint = within(main()).getByRole("link", { name: "Create a part on the project page" });
    expect(hint.getAttribute("href")).toBe(`${PROJECT_PATH}/parts`);
  });

  test("the filter narrows the list as you type to the names and full descriptions holding the trimmed query, in the same order, and says N of M", async () => {
    fakeApi(home({ tracks }));
    renderAt(`${PROJECT_PATH}/tracks`);
    await waitFor(() => expect(rows()).toHaveLength(5));

    filter("  WALLET ");
    expect(rowNames()).toEqual(["Checkout", "Wallets"]);
    expect(within(main()).getByText("2 of 5")).toBeTruthy();

    filter("  WALLETS a");
    expect(within(main()).queryAllByRole("link")).toEqual([]);
    expect(within(main()).getByText("0 of 5")).toBeTruthy();

    filter("");
    expect(rows()).toHaveLength(5);
    expect(within(main()).getByText("5 tracks")).toBeTruthy();
  });
});

describe("the palette", () => {
  const payHome = home({
    tracks: [track(PAYMENTS, "Payments"), track(PAY_LATER, "Pay later"), track(CHECKOUT, "Checkout")],
    docs: [1, 2, 3, 4, 5, 6, 7].map((n) => doc(`0192d7a0-5555-7000-8000-00000000002${n}`, null, `Pay note ${n}`)),
    threads: [thread(CHECKOUT_THREAD, CHECKOUT, "Which wallets?")],
  });

  /** Renders the Tracks page on `platform`, and waits until the top bar has the project home. */
  async function renderOn(platform: string, projectHome: Home) {
    Object.defineProperty(navigator, "platform", { value: platform, configurable: true });
    fakeApi(projectHome);
    const router = renderAt(`${PROJECT_PATH}/tracks`);
    const pills = await screen.findByRole("navigation", { name: "Storefront" });
    await within(pills).findByRole("link", { name: /^Tracks\d/ });
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

  test("the Search button opens it on the places visited last, a doc with its scope and pillar, a thread with its scope", async () => {
    const visits = [{ scope: CHECKOUT, doc: CHECKOUT_DOC }, { scope: CHECKOUT, thread: CHECKOUT_THREAD }, { scope: "app" }];
    localStorage.setItem(VISITS_KEY, JSON.stringify(visits.map((visit, n) => ({ ...visit, at: Date.parse(AT) - n * 60_000 }))));
    await renderOn("MacIntel", home({ tracks: [track(CHECKOUT, "Checkout")], docs: [doc(CHECKOUT_DOC, CHECKOUT, "Pay in one step")], threads: [thread(CHECKOUT_THREAD, CHECKOUT, "Which wallets?")] }));

    fireEvent.click(screen.getByRole("button", { name: /^Search/ }));

    expect(options().map(shownText)).toEqual(["Pay in one stepCheckout · Spec", "Which wallets?Checkout", "App"]);
  });

  test("typing groups the matches as Tracks and Docs, says how many a full group shows of how many, and leaves out an empty group", async () => {
    await renderOn("MacIntel", payHome);
    press("k", { metaKey: true });
    type("pay");

    const groups = within(palette()!).getAllByRole("group");
    expect(groups.map((group) => group.getAttribute("aria-label"))).toEqual(["Tracks", "Docs"]);
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
    await waitFor(() => expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/tracks/${PAYMENTS}`));
    expect(palette()).toBeNull();
  });
});
