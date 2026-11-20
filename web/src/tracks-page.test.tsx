// @vitest-environment jsdom
// The Tracks of a project and of a part (slice S2; prototype
// tasks/ui-redesign/v2/tracks.html): grouped by state like Linear, a filter on
// state, owner and part, and Start track. Then the palette that the header's
// Search button or ⌘K (Ctrl+K off macOS) opens (US-125). Both read the project
// home, GET .../projects/<project>/home. The app is rendered into the
// document, as in the browser: the palette's portal on the body then sees the
// same events as there.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import * as scope from "./scope-pages-fake";
import { choose } from "./testing/select";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
/** The part every track of the fixtures is on (US-305). */
const PART_ID = "0192d7a0-3333-7000-8000-000000000001";
const CHECKOUT = "0192d7a0-4444-7000-8000-000000000001";
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
          return json(200, { actor: { kind: "human", user_id: USER.id }, workspace_id: WORKSPACE_ID, workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
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
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-11-17T12:00:00Z"));
  });

  const page = () => screen.findByRole("main");
  /** The bands of the page: each one's name and count, and its rows as the reader sees them. */
  const bands = async () =>
    within(await page())
      .getAllByRole("region")
      .map((band): [string | null, string[]] => [band.getAttribute("aria-label"), within(band).getAllByRole("listitem").map(scope.shownText)]);

  test("tracks come in bands by state, Planning, Building, Done, Dropped, each with the most recently active first; a state with none has no band", async () => {
    const wision = scope.part("Wision", "app");
    scope.fakeScopeApi({
      parts: [wision],
      tracks: [
        scope.track("Charts", wision.id, "done", { lastActivity: "2026-03-06T08:00:00Z" }),
        scope.track("Alarms", wision.id, "done", { lastActivity: "2026-11-04T08:00:00Z" }),
        scope.track("Exports", wision.id, "planning", { lastActivity: null, createdAt: "2026-11-17T11:00:00Z" }),
        scope.track("Long windows", wision.id, "planning", { lastActivity: "2026-11-17T11:48:00Z" }),
        scope.track("Moves", wision.id, "dropped", { lastActivity: "2026-11-16T08:00:00Z" }),
      ],
    });
    renderAt(`${scope.PROJECT_PATH}/tracks`);
    await within(await page()).findByText("Long windows");
    expect((await bands()).map(([name, rows]) => [name, rows.map((row) => row.split(/ change[sd] /)[0])])).toEqual([
      ["Planning", ["Long windows", "Exports"]],
      ["Done", ["Alarms", "Charts"]],
      ["Dropped", ["Moves"]],
    ]);
    expect(within(await page()).getByText("5 tracks")).toBeTruthy();
  });

  test("a row shows what the track changes, one signal (its questions), its part on the project's page and when it last moved, and opens the track", async () => {
    const sara = scope.member("sara.lind@example.com");
    const wision = scope.part("Wision", "app");
    const iot = scope.part("iot-service", "service");
    const readings = scope.feature(iot.id, "Readings query");
    const windows = scope.track("Long windows", wision.id, "planning", { owner: scope.me.user_id, total: 2, lastActivity: "2026-11-17T11:48:00Z" });
    const query = scope.track("Aggregated readings", iot.id, "building", {
      owner: sara.user_id,
      done: 1,
      total: 2,
      lastActivity: "2026-11-04T08:00:00Z",
      splitFrom: windows.track.id,
    });
    scope.fakeScopeApi({
      members: [scope.me, sara],
      parts: [wision, iot],
      features: [readings],
      tracks: [windows, query],
      changes: { [readings.id]: [query.track.id] },
      questions: { [windows.track.id]: 3 },
    });
    renderAt(`${scope.PROJECT_PATH}/tracks`);
    await waitFor(async () => expect((await bands())[0][1][0]).toContain("3 questions"));
    expect(await bands()).toEqual([
      ["Planning", ["Long windows changes no feature · split off to Aggregated readings 3 questions Wision 12 min"]],
      ["Building", ["Aggregated readings changes Readings query · split off from Long windows iot-service 4 Nov"]],
    ]);
    const row = within(await page()).getByRole("link", { name: /^BuildingAggregated readings/ });
    expect(row.getAttribute("href")).toBe(`${scope.PROJECT_PATH}/tracks/${query.track.id}`);

    cleanup();
    renderAt(`${scope.PROJECT_PATH}/parts/${iot.id}/tracks`);
    await within(await page()).findByText("Aggregated readings");
    expect(await bands(), "a part's page leaves out the part, and other parts' tracks").toEqual([
      ["Building", ["Aggregated readings changes Readings query · split off from Long windows 4 Nov"]],
    ]);
  });

  test("the filter narrows by state, owner and part, and says N of M tracks", async () => {
    const sara = scope.member("sara.lind@example.com");
    const wision = scope.part("Wision", "app");
    const iot = scope.part("iot-service", "service");
    scope.fakeScopeApi({
      members: [scope.me, sara],
      parts: [wision, iot],
      tracks: [
        scope.track("Long windows", wision.id, "planning", { owner: scope.me.user_id }),
        scope.track("Exports", wision.id, "planning", { owner: sara.user_id }),
        scope.track("Query", iot.id, "planning", { owner: sara.user_id }),
        scope.track("Ingest", iot.id, "done", { owner: sara.user_id }),
      ],
    });
    renderAt(`${scope.PROJECT_PATH}/tracks`);
    const filters = within(await page()).getByRole("group", { name: "Filter" });
    await within(await page()).findByText("Query");
    choose(within(filters).getByRole("combobox", { name: "State" }), "Planning");
    choose(within(filters).getByRole("combobox", { name: "Owner" }), "Sara");
    choose(within(filters).getByRole("combobox", { name: "Part" }), "Wision");
    expect((await bands()).map(([name, rows]) => [name, rows.map((row) => row.split(" changes")[0])])).toEqual([["Planning", ["Exports"]]]);
    expect(within(filters).getByText("1 of 4 tracks")).toBeTruthy();
  });

  test("Start track asks for a part, starting empty, and creates the track on the part picked; on a part's Tracks it goes on that part", async () => {
    const wision = scope.part("Wision", "app");
    const iot = scope.part("iot-service", "service");
    const api = scope.fakeScopeApi({ parts: [wision, iot] });
    renderAt(`${scope.PROJECT_PATH}/tracks`);
    fireEvent.click(await within(await page()).findByRole("button", { name: "Start track" }));
    const dialog = screen.getByRole("dialog", { name: "Start a track" });
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Name" }), { target: { value: "Gateway" } });
    const part = within(dialog).getByRole("combobox", { name: "Part" });
    expect(part.textContent, "no part chosen").toBe("Choose a part");
    fireEvent.click(within(dialog).getByRole("button", { name: "Start track" }));
    expect(within(dialog).getByRole("alert").textContent).toBe("Choose the part it changes");
    expect(api.writes, "no part, no track").toEqual([]);
    choose(part, /^iot-service/);
    fireEvent.click(within(dialog).getByRole("button", { name: "Start track" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "POST", path: `${scope.API}/tracks`, body: { name: "Gateway", description: "", part_id: iot.id } }]));
    expect(await within(await page()).findByText("Gateway")).toBeTruthy();

    cleanup();
    api.writes.length = 0;
    renderAt(`${scope.PROJECT_PATH}/parts/${wision.id}/tracks`);
    fireEvent.click(await within(await page()).findByRole("button", { name: "Start track" }));
    const onPart = screen.getByRole("dialog", { name: "Start a track on Wision" });
    expect(within(onPart).queryByRole("combobox", { name: "Part" })).toBeNull();
    fireEvent.change(within(onPart).getByRole("textbox", { name: "Name" }), { target: { value: "Exports" } });
    fireEvent.click(within(onPart).getByRole("button", { name: "Start track" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "POST", path: `${scope.API}/tracks`, body: { name: "Exports", description: "", part_id: wision.id } }]));
  });

  test("with no parts, Start track says a track goes on a part and points to Parts", async () => {
    scope.fakeScopeApi({ parts: [] });
    renderAt(`${scope.PROJECT_PATH}/tracks`);
    fireEvent.click(await within(await page()).findByRole("button", { name: "Start track" }));
    const status = await within(screen.getByRole("dialog", { name: "Start a track" })).findByRole("status");
    expect(within(status).getByRole("link", { name: "Create a part" }).getAttribute("href")).toBe(`${scope.PROJECT_PATH}/parts`);
  });
});

describe("the palette", () => {
  const payHome = home({
    tracks: [track(PAYMENTS, "Payments"), track(PAY_LATER, "Pay later"), track(CHECKOUT, "Checkout")],
    docs: [1, 2, 3, 4, 5, 6, 7].map((n) => doc(`0192d7a0-5555-7000-8000-00000000002${n}`, null, `Pay note ${n}`)),
    threads: [thread(CHECKOUT_THREAD, CHECKOUT, "Which wallets?")],
  });

  /** Renders the Tracks page on `platform`, and waits until the shell has the project home. */
  async function renderOn(platform: string, projectHome: Home) {
    Object.defineProperty(navigator, "platform", { value: platform, configurable: true });
    fakeApi(projectHome);
    const router = renderAt(`${PROJECT_PATH}/tracks`);
    const sections = await screen.findByRole("navigation", { name: "Sections" });
    await within(sections).findByRole("link", { name: /^Tracks, \d/ });
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

    expect(options().map(shownText)).toEqual(["Pay in one stepCheckout · Spec", "Which wallets?Checkout", "Storefront"]);
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
      "Pay note 1Storefront · Spec",
      "Pay note 2Storefront · Spec",
      "Pay note 3Storefront · Spec",
      "Pay note 4Storefront · Spec",
      "Pay note 5Storefront · Spec",
      "Pay note 6Storefront · Spec",
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
    expect(selected()).toEqual(["Pay note 1Storefront · Spec"]);
    fireEvent.keyDown(input(), { key: "ArrowUp" });
    expect(selected()).toEqual(["Payments"]);

    fireEvent.keyDown(input(), { key: "Enter" });
    await waitFor(() => expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/tracks/${PAYMENTS}`));
    expect(palette()).toBeNull();
  });
});
