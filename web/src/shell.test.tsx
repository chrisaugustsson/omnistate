// @vitest-environment jsdom
// The shell of navigation F (tasks/ui-redesign/build-f.md): a path header
// (workspace ⌄ / project ⌄ / part ⌄ / feature ⌄ or track ⌄) whose names lead
// to each level's first section and whose ⌄ lists the siblings, keeping the
// section; a sidebar of the last step's sections with their counts, dark where
// something waits on the reader, folded on the canvas sections; on the phone a
// section button and sheets instead; and old URLs that lead to their new
// place. The app is rendered with its own router on a memory history and the
// browser's fetch is replaced by a fake of the API.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WS = "0192d7a0-0000-7000-8000-00000000000a";
const ME = "0192d7a0-1111-7000-8000-000000000001";
const SARA = "0192d7a0-1111-7000-8000-000000000002";
const MONITORING = "0192d7a0-2222-7000-8000-000000000001";
const INSTALLER = "0192d7a0-2222-7000-8000-000000000002";
const WISION = "0192d7a0-3333-7000-8000-000000000001";
const IOT = "0192d7a0-3333-7000-8000-000000000002";
const FIELD_APP = "0192d7a0-3333-7000-8000-000000000003";
const ANALYZE = "0192d7a0-5555-7000-8000-000000000001";
const ALERTS = "0192d7a0-5555-7000-8000-000000000002";
const LONG_WINDOWS = "0192d7a0-4444-7000-8000-000000000001";
const AGGREGATES = "0192d7a0-4444-7000-8000-000000000002";
const ALARMS = "0192d7a0-4444-7000-8000-000000000003";
const THRESHOLDS = "0192d7a0-4444-7000-8000-000000000004";
const THREAD = "0192d7a0-6666-7000-8000-000000000001";
const TRACK_DOC = "0192d7a0-7777-7000-8000-000000000001";
const AT = "2026-11-10T09:00:00Z";
const P = `/w/${WS}/projects/${MONITORING}`;
const API = `/api/v1/w/${WS}`;

const part = (id: string, project: string, name: string, kind: string, created: string) => ({
  id,
  project_id: project,
  name,
  kind,
  description: "",
  created_at: created,
});

const track = (id: string, partId: string, name: string, state: string, owner: string, splitFrom: string | null = null) => ({
  track: { id, project_id: MONITORING, part_id: partId, name, description: "", state, owner_user_id: owner, split_from_track_id: splitFrom, created_at: AT },
  docs: 0,
  waiting: 0,
  stories: { total: 2, open: 2, done: 0 },
  last_activity: null,
});

const waiting = (id: string, trackId: string) => ({
  id,
  project_id: MONITORING,
  track_id: trackId,
  doc_id: TRACK_DOC,
  node: "p0000001",
  author: { kind: "human", user_id: ME },
  text: "Is a year enough?",
  replies: [{ id: `${id}-r`, author: { kind: "agent", role: "writer", run: "r1" }, text: "Yes.", created_at: AT }],
  orphaned: false,
  waiting_on_you: true,
  created_at: AT,
});

/**
 * Monitoring platform: Wision (app, oldest) and iot-service. On Wision, Christopher's
 * track in flight and a done one, and Sara's dropped one; on iot-service, Sara's
 * track split off from Christopher's.
 */
function monitoringHome() {
  return {
    project_id: MONITORING,
    app: { last_activity: null },
    tracks: [
      track(LONG_WINDOWS, WISION, "Long time windows in Analyze", "planning", ME),
      track(AGGREGATES, IOT, "Aggregated readings query", "building", SARA, LONG_WINDOWS),
      track(ALARMS, WISION, "Alarm thresholds per channel", "done", ME),
      track(THRESHOLDS, WISION, "Threshold presets", "dropped", SARA),
    ],
    docs: [
      { id: "0192d7a0-7777-7000-8000-000000000002", project_id: MONITORING, kind: "html", pillar: "architecture", title: "Architecture overview", created_at: AT },
      { id: TRACK_DOC, project_id: MONITORING, track_id: LONG_WINDOWS, kind: "html", pillar: "spec", title: "Long windows spec", created_at: AT },
      { id: "0192d7a0-7777-7000-8000-000000000003", project_id: MONITORING, feature_id: ANALYZE, kind: "html", pillar: "spec", title: "How Analyze works", created_at: AT },
    ],
    threads: [],
    // One comment waits on Christopher's track, one on Sara's.
    waiting: { comments: [waiting("0192d7a0-8888-7000-8000-000000000001", LONG_WINDOWS), waiting("0192d7a0-8888-7000-8000-000000000002", AGGREGATES)] },
  };
}

function fakeApi() {
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const feature = (id: string, name: string) => ({ id, project_id: MONITORING, part_id: WISION, name, description: "", created_at: AT });
  const routes: Record<string, unknown> = {
    "/api/v1/auth/me": { user: { id: ME, email: "chris.doe@example.com" } },
    "/api/v1/auth/workspaces": { workspaces: [{ id: WS, name: "Wioniq" }] },
    [`${API}/whoami`]: { actor: { kind: "human", user_id: ME }, workspace_id: WS },
    [`${API}/members`]: { members: [{ membership_id: "m1", user_id: ME, role: "admin", email: "chris.doe@example.com" }] },
    [`${API}/projects`]: {
      projects: [
        { id: MONITORING, name: "Monitoring platform", created_at: AT },
        { id: INSTALLER, name: "Installer app", created_at: AT },
      ],
    },
    [`${API}/projects/${MONITORING}/parts`]: {
      parts: [part(IOT, MONITORING, "iot-service", "service", "2026-11-02T00:00:00Z"), part(WISION, MONITORING, "Wision", "app", "2026-11-01T00:00:00Z")],
    },
    [`${API}/projects/${INSTALLER}/parts`]: { parts: [part(FIELD_APP, INSTALLER, "Field app", "app", AT)] },
    [`${API}/projects/${MONITORING}/home`]: monitoringHome(),
    [`${API}/projects/${INSTALLER}/home`]: { project_id: INSTALLER, tracks: [], docs: [], threads: [], waiting: { comments: [] } },
    [`${API}/projects/${MONITORING}/parts/${WISION}/features`]: { features: [feature(ANALYZE, "Analyze"), feature(ALERTS, "Alerts")] },
    [`${API}/projects/${MONITORING}/parts/${WISION}/features/${ANALYZE}`]: { feature: feature(ANALYZE, "Analyze") },
    [`${API}/projects/${MONITORING}/threads/${THREAD}`]: {
      thread: { id: THREAD, project_id: MONITORING, track_id: LONG_WINDOWS, question: "Which presets?", sealed: false, created_at: AT, posts: [] },
    },
    [`${API}/projects/${MONITORING}/docs/${TRACK_DOC}`]: {
      doc: { id: TRACK_DOC, project_id: MONITORING, track_id: LONG_WINDOWS, kind: "html", pillar: "spec", title: "Long windows spec", created_at: AT },
    },
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = new URL(url, "http://localhost:3000").pathname;
      return path in routes ? json(200, routes[path]) : json(404, { error: "not_found" });
    }),
  );
}

/** Makes the window phone-wide (under 768px) or not. */
function windowWidth(phone: boolean) {
  vi.stubGlobal("matchMedia", (media: string) => ({ media, matches: phone, addEventListener() {}, removeEventListener() {} }));
}

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

const breadcrumb = () => screen.findByRole("navigation", { name: "Breadcrumb" });

/** Each step of the path: its name and where it leads. */
async function steps() {
  return within(await breadcrumb())
    .getAllByRole("link")
    .map((link) => [link.textContent, link.getAttribute("href")]);
}

/** Opens the ⌄ menu of the step that switches `noun` ("part"), once what it lists has loaded. */
async function openMenu(noun: string, loaded: string | RegExp) {
  fireEvent.click(await within(await breadcrumb()).findByRole("button", { name: `Switch ${noun}` }));
  const menu = await screen.findByRole("dialog", { name: `Switch ${noun}` });
  await within(menu).findByText(loaded);
  return menu;
}

/** A menu's rows as their text. */
const rows = (menu: HTMLElement) => [...within(menu).queryAllByRole("link"), ...within(menu).queryAllByRole("button")].map((row) => row.textContent);

/** The row of a menu whose name matches. */
const row = (menu: HTMLElement, name: RegExp) => within(menu).getByRole("link", { name });

/** The sidebar's sections, each by its name (with its count and whether it waits on you) and whether it is lit. */
async function sections() {
  const nav = await screen.findByRole("navigation", { name: "Sections" });
  return within(nav)
    .getAllByRole("link")
    .map((link) => [link.getAttribute("aria-label"), link.getAttribute("aria-current") === "page"]);
}

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  windowWidth(false);
  fakeApi();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  localStorage.clear();
});

describe("the path", () => {
  test("on a track, it is the workspace, the project, the track's part and the track, each name leading to its level's first section", async () => {
    renderAt(`${P}/tracks/${LONG_WINDOWS}/board`);

    await waitFor(async () =>
      expect(await steps()).toEqual([
        ["Wioniq", `/w/${WS}`],
        ["Monitoring platform", P],
        ["WWision", `${P}/parts/${WISION}`],
        ["Long time windows in Analyze", `${P}/tracks/${LONG_WINDOWS}`],
      ]),
    );
  });

  test("on a feature, it ends with the feature after its part, leading to its Overview", async () => {
    renderAt(`${P}/parts/${WISION}/features/${ANALYZE}/decisions`);

    await waitFor(async () => expect((await steps()).slice(2)).toEqual([["WWision", `${P}/parts/${WISION}`], ["Analyze", `${P}/parts/${WISION}/features/${ANALYZE}`]]));
  });

  test("on the workspace, it is the workspace alone", async () => {
    renderAt(`/w/${WS}/my-tracks`);

    await waitFor(async () => expect(await steps()).toEqual([["Wioniq", `/w/${WS}`]]));
  });

  test("the project's menu lists the workspace's projects with their parts, and keeps the section", async () => {
    renderAt(`${P}/decisions`);
    const menu = await openMenu("project", "2 parts");

    await waitFor(() => expect(rows(menu)).toEqual(["Monitoring platform2 parts", "Installer app1 part"]));
    expect(row(menu, /Installer app/).getAttribute("href")).toBe(`/w/${WS}/projects/${INSTALLER}/decisions`);
    expect(within(menu).getByText(/You stay on/).textContent).toBe("You stay on Decisions");
  });

  test("the part's menu lists Project-wide, then the parts oldest first with their kind and the current one checked, then New part", async () => {
    renderAt(`${P}/parts/${IOT}`);
    const menu = await openMenu("part", "Wision");

    expect(rows(menu)).toEqual(["Project-wideall parts", "WWisionapp", "Iiot-serviceservice", "New part"]);
    expect(row(menu, /iot-service/).getAttribute("aria-current")).not.toBeNull();
    expect(row(menu, /Wision/).getAttribute("aria-current")).toBeNull();
  });

  test("switching part keeps the section: from a part's Tracks every part opens its Tracks and Project-wide the project's", async () => {
    renderAt(`${P}/parts/${WISION}/tracks`);
    const menu = await openMenu("part", "iot-service");

    expect(row(menu, /iot-service/).getAttribute("href")).toBe(`${P}/parts/${IOT}/tracks`);
    expect(row(menu, /Project-wide/).getAttribute("href")).toBe(`${P}/tracks`);
    expect(within(menu).getByText(/You stay on/).textContent).toBe("You stay on Tracks");
  });

  test("from a feature the parts open their Features and Project-wide the project's Parts; from a track, their Tracks", async () => {
    renderAt(`${P}/parts/${WISION}/features/${ANALYZE}`);
    let menu = await openMenu("part", "iot-service");
    expect(row(menu, /iot-service/).getAttribute("href")).toBe(`${P}/parts/${IOT}/features`);
    expect(row(menu, /Project-wide/).getAttribute("href")).toBe(`${P}/parts`);
    cleanup();

    renderAt(`${P}/tracks/${LONG_WINDOWS}/how`);
    menu = await openMenu("part", "iot-service");
    expect(row(menu, /iot-service/).getAttribute("href")).toBe(`${P}/parts/${IOT}/tracks`);
  });

  test("the feature's menu lists the part's features, each on the same section", async () => {
    renderAt(`${P}/parts/${WISION}/features/${ANALYZE}/tracks`);
    const menu = await openMenu("feature", "Alerts");

    expect(rows(menu)).toEqual(["Analyze", "Alerts"]);
    expect(row(menu, /Alerts/).getAttribute("href")).toBe(`${P}/parts/${WISION}/features/${ALERTS}/tracks`);
  });

  test("the track's menu lists the part's tracks in flight, then done and dropped, then a track split off on another part, then Start track; each keeps the section", async () => {
    renderAt(`${P}/tracks/${LONG_WINDOWS}/board`);
    const menu = await openMenu("track", "Threshold presets");

    expect(rows(menu)).toEqual([
      "Long time windows in Analyze",
      "Alarm thresholds per channel",
      "Threshold presets",
      "Split off: Aggregated readings queryiot-service",
      "Start track",
    ]);
    expect(row(menu, /Alarm thresholds/).getAttribute("href")).toBe(`${P}/tracks/${ALARMS}/board`);
    expect(row(menu, /Split off/).getAttribute("href")).toBe(`${P}/tracks/${AGGREGATES}/board`);
  });

  test("typing narrows the menu to the names and kinds that hold it, ↓ goes to the current row, and Esc closes it", async () => {
    renderAt(`${P}/parts/${WISION}`);
    const menu = await openMenu("part", "iot-service");
    const find = within(menu).getByRole("textbox", { name: "Find a part" });

    fireEvent.keyDown(find, { key: "ArrowDown" });
    expect(document.activeElement?.textContent).toBe("WWisionapp");
    fireEvent.change(find, { target: { value: "SERVICE" } });
    expect(rows(menu)).toEqual(["Iiot-serviceservice"]);
    fireEvent.keyDown(find, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Switch part" })).toBeNull();
  });

  test("the Inbox counts the comments waiting on tracks you own, across the workspace, and leads to the workspace", async () => {
    renderAt(P);

    // Two comments wait in Monitoring platform, one of them on Sara's track.
    const inbox = await screen.findByRole("link", { name: "Inbox, 1 waiting" });
    expect(inbox.getAttribute("href")).toBe(`/w/${WS}`);
  });
});

describe("the sections sidebar", () => {
  test("on the workspace: Inbox with what waits on you as a dark count, My tracks, Projects", async () => {
    renderAt(`/w/${WS}/my-tracks`);

    await waitFor(async () =>
      expect(await sections()).toEqual([
        ["Inbox, 1 waiting on you", false],
        ["My tracks, 1", true],
        ["Projects, 2", false],
      ]),
    );
  });

  test("on a project: its parts, its tracks in flight and its own docs counted", async () => {
    renderAt(`${P}/decisions`);

    await waitFor(async () =>
      expect(await sections()).toEqual([
        ["Overview", false],
        ["Parts, 2", false],
        ["Tracks, 2", false],
        ["Docs, 1", false],
        ["Decisions", true],
      ]),
    );
    expect((await screen.findByRole("link", { name: /^Parts/ })).getAttribute("href")).toBe(`${P}/parts`);
  });

  test("on a part: its features, its tracks in flight, and its docs as its Docs lists them (its features' too)", async () => {
    renderAt(`${P}/parts/${WISION}/features`);

    await waitFor(async () =>
      expect(await sections()).toEqual([
        ["Overview", false],
        ["Features, 2", true],
        ["Tracks, 1", false],
        ["Docs, 1", false],
        ["Decisions", false],
      ]),
    );
  });

  test("on a feature: Overview, Docs, Tracks and Decisions, Design left out while it has no design doc", async () => {
    renderAt(`${P}/parts/${WISION}/features/${ANALYZE}/docs`);

    await waitFor(async () => expect((await sections()).map(([name]) => name)).toEqual(["Overview", "Docs, 1", "Tracks", "Decisions"]));
    expect((await sections())[1][1]).toBe(true);
  });

  test("on your track: Threads is a dark count while a comment waits on you, and each section leads to the track's page of it", async () => {
    renderAt(`${P}/tracks/${LONG_WINDOWS}`);

    await waitFor(async () =>
      expect(await sections()).toEqual([
        ["Spec", true],
        ["Behaviour", false],
        ["Design", false],
        ["How", false],
        ["Board, 0/2", false],
        ["Threads, 1 waiting on you", false],
        ["Decisions", false],
      ]),
    );
    expect(screen.getByRole("link", { name: /^Behaviour/ }).getAttribute("href")).toBe(`${P}/tracks/${LONG_WINDOWS}/behaviour`);
  });

  test("on someone else's track the comment waiting there is not yours, and its service part has no Design", async () => {
    renderAt(`${P}/tracks/${AGGREGATES}`);

    await waitFor(async () => expect((await sections()).map(([name]) => name)).toEqual(["Spec", "Behaviour", "How", "Board, 0/2", "Threads", "Decisions"]));
  });

  test("folds to icons on a canvas section, and » brings the names back", async () => {
    renderAt(`${P}/tracks/${LONG_WINDOWS}/board`);
    const board = await screen.findByRole("link", { name: /^Board/ });

    expect(board.textContent).not.toContain("Board");
    fireEvent.click(screen.getByRole("button", { name: "Show section names" }));
    expect(screen.getByRole("link", { name: /^Board/ }).textContent).toContain("Board");
  });

  test("folds at your word on any page, and stays folded on the next", async () => {
    const router = renderAt(`${P}/parts`);
    fireEvent.click(await screen.findByRole("button", { name: "Fold the sidebar" }));
    expect(screen.getByRole("link", { name: /^Parts/ }).textContent).not.toContain("Parts");

    await router.navigate({ href: `${P}/docs` });
    expect((await screen.findByRole("link", { name: /^Docs/ })).textContent).not.toContain("Docs");
  });
});

describe("on the phone", () => {
  beforeEach(() => windowWidth(true));

  test("a section button names the open section with what waits on you, and opens the same sections as a sheet", async () => {
    renderAt(`${P}/tracks/${LONG_WINDOWS}/how`);
    const button = await screen.findByRole("button", { name: "Sections: How, 1 waiting on you" });

    expect(screen.queryByRole("navigation", { name: "Sections" })).toBeNull();
    fireEvent.click(button);
    const sheet = await screen.findByRole("dialog", { name: "Sections" });
    expect(within(sheet).getAllByRole("link").map((link) => link.textContent)).toEqual([
      "Spec",
      "Behaviour",
      "Design",
      "How",
      "Board0 of 2",
      "Threads1 for you",
      "Decisions",
    ]);
  });

  test("the header leads up to the part's Tracks, and the track's name opens its siblings under the path", async () => {
    renderAt(`${P}/tracks/${LONG_WINDOWS}`);

    expect((await screen.findByRole("link", { name: "Up to Wision" })).getAttribute("href")).toBe(`${P}/parts/${WISION}/tracks`);
    fireEvent.click(await screen.findByRole("button", { name: /Long time windows in Analyze/ }));
    const sheet = await screen.findByRole("dialog", { name: "Switch track" });
    expect(within(within(sheet).getByRole("navigation", { name: "Path" })).getAllByRole("link").map((link) => link.textContent)).toEqual([
      "Wioniq",
      "Monitoring platform",
      "Wision",
    ]);
    expect(within(sheet).getByRole("link", { name: /Alarm thresholds/ }).getAttribute("href")).toBe(`${P}/tracks/${ALARMS}`);
  });
});

describe("old URLs", () => {
  test.each([
    ["the App page", `${P}/app`, P],
    ["the Inbox", `/w/${WS}/inbox`, `/w/${WS}`],
    ["a project's Inbox", `${P}/inbox`, `/w/${WS}`],
    ["a project's My tracks", `${P}/my-tracks`, `/w/${WS}/my-tracks`],
    ["Threads narrowed to a track", `${P}/threads?scope=${LONG_WINDOWS}`, `${P}/tracks/${LONG_WINDOWS}/threads`],
    ["a track's thread", `${P}/threads/${THREAD}`, `${P}/tracks/${LONG_WINDOWS}/threads/${THREAD}`],
    ["a track's doc", `${P}/docs/${TRACK_DOC}`, `${P}/tracks/${LONG_WINDOWS}/docs/${TRACK_DOC}`],
  ])("%s leads to its new place", async (_, old, now) => {
    const router = renderAt(old);
    await waitFor(() => expect(router.state.location.pathname).toBe(now));
  });

  test("the Inbox keeps the comment to open on its way to the workspace", async () => {
    const router = renderAt(`/w/${WS}/inbox?comment=c1`);
    await waitFor(() => expect(router.state.location.pathname).toBe(`/w/${WS}`));
    expect(router.state.location.search).toEqual({ comment: "c1" });
  });

  test("a doc link keeps where in the doc to be on its way to the new place", async () => {
    const router = renderAt(`${P}/docs/${TRACK_DOC}?node=p0000001`);
    await waitFor(() => expect(router.state.location.pathname).toBe(`${P}/tracks/${LONG_WINDOWS}/docs/${TRACK_DOC}`));
    expect(router.state.location.search).toEqual({ node: "p0000001" });
  });
});
