// @vitest-environment jsdom
// The shell of the UI redesign (tasks/ui-redesign/build-plan.md, "URL map";
// v2/build-brief-d.md, "The shell"): one top bar with the scope button, the
// scope's pills, Search, the Inbox and the account menu; a switcher that keeps
// the tab; object pages that keep their part in the bar; old URLs that lead to
// their new place. The app is rendered with its own router on a memory history
// and the browser's fetch is replaced by a fake of the API.
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
const LONG_WINDOWS = "0192d7a0-4444-7000-8000-000000000001";
const AGGREGATES = "0192d7a0-4444-7000-8000-000000000002";
const ALARMS = "0192d7a0-4444-7000-8000-000000000003";
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

const track = (id: string, partId: string, name: string, state: string, owner: string) => ({
  track: { id, project_id: MONITORING, part_id: partId, name, description: "", state, owner_user_id: owner, created_at: AT },
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

/** Monitoring platform: Wision (app) and iot-service; three tracks, two in flight on Wision's side and one done. */
function monitoringHome() {
  return {
    project_id: MONITORING,
    app: { last_activity: null },
    tracks: [
      track(LONG_WINDOWS, WISION, "Long time windows in Analyze", "planning", ME),
      track(AGGREGATES, IOT, "Aggregated readings query", "building", SARA),
      track(ALARMS, WISION, "Alarm thresholds per channel", "done", ME),
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
  const routes: Record<string, unknown> = {
    "/api/v1/auth/me": { user: { id: ME, email: "chris.doe@example.com" } },
    "/api/v1/auth/workspaces": { workspaces: [{ id: WS, name: "Wioniq" }] },
    [`${API}/whoami`]: { actor: { kind: "human", user_id: ME }, workspace_id: WS },
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
    [`${API}/projects/${MONITORING}/parts/${WISION}/features`]: {
      features: [{ id: ANALYZE, project_id: MONITORING, part_id: WISION, name: "Analyze", description: "", created_at: AT }],
    },
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

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

/** The scope's pills, once they show; named for the scope. */
const pills = (scope: string) => screen.findByRole("navigation", { name: scope });

/** Each pill's text (label and count) and whether it is lit. */
function pillStates(nav: HTMLElement) {
  return within(nav)
    .getAllByRole("link")
    .map((pill) => [pill.textContent, pill.getAttribute("aria-current") === "page"]);
}

const scopeButton = () => screen.getByRole("button", { expanded: false, name: /Monitoring platform|Home/ });

async function openSwitcher() {
  fireEvent.click(scopeButton());
  const switcher = await screen.findByRole("dialog", { name: "Switch project or part" });
  // The parts of every project load when the switcher opens.
  await within(switcher).findByText("2 parts");
  return switcher;
}

describe("the top bar", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    fakeApi();
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  test("a project page shows the project under the workspace, and its pills with parts, project docs and tracks in flight counted", async () => {
    renderAt(`${P}/decisions`);
    const nav = await pills("Monitoring platform");

    await waitFor(() =>
      expect(pillStates(nav)).toEqual([
        ["Overview", false],
        ["Parts2", false],
        ["Docs1", false],
        ["Tracks2", false],
        ["Decisions", true],
      ]),
    );
    expect(scopeButton().textContent).toBe("WioniqMonitoring platform");
    expect(within(nav).getByRole("link", { name: /^Parts/ }).getAttribute("href")).toBe(`${P}/parts`);
  });

  test("a part page shows the part under its project, its features counted, its docs as its Docs tab lists them (its features' too) and only its tracks in flight", async () => {
    renderAt(`${P}/parts/${WISION}/features`);
    const nav = await pills("Wision");

    await waitFor(() =>
      expect(pillStates(nav)).toEqual([
        ["Overview", false],
        ["Features1", true],
        ["Docs1", false],
        ["Tracks1", false],
        ["Decisions", false],
      ]),
    );
    expect(scopeButton().textContent).toBe("WMonitoring platformWision");
  });

  test("a track page keeps the track's part in the bar with Tracks lit, its pill leading to the part's tracks", async () => {
    renderAt(`${P}/tracks/${AGGREGATES}/board`);
    const nav = await pills("iot-service");

    const tracks = within(nav).getByRole("link", { name: /^Tracks/ });
    expect(tracks.getAttribute("aria-current")).toBe("page");
    expect(tracks.getAttribute("href")).toBe(`${P}/parts/${IOT}/tracks`);
    const path = screen.getByRole("navigation", { name: "Path" });
    expect(within(path).getByRole("link", { name: "Tracks" }).getAttribute("href")).toBe(`${P}/parts/${IOT}/tracks`);
  });

  test("a feature page keeps its part in the bar with Features lit", async () => {
    renderAt(`${P}/parts/${WISION}/features/${ANALYZE}/decisions`);
    const nav = await pills("Wision");

    expect(within(nav).getByRole("link", { name: /^Features/ }).getAttribute("aria-current")).toBe("page");
    expect(within(nav).getAllByRole("link").filter((pill) => pill.getAttribute("aria-current") === "page")).toHaveLength(1);
  });

  test("Home shows Home under the workspace with For you, Inbox and My tracks", async () => {
    renderAt(`/w/${WS}/my-tracks`);
    const nav = await pills("Home");

    expect(scopeButton().textContent).toBe("CWioniqHome");
    expect(pillStates(nav).map(([label, lit]) => [String(label).replace(/\d+$/, ""), lit])).toEqual([
      ["For you", false],
      ["Inbox", false],
      ["My tracks", true],
    ]);
  });

  test("the Inbox counts the comments waiting on tracks you own, across the workspace", async () => {
    renderAt(`${P}`);
    // Two comments wait in Monitoring platform, one of them on Sara's track.
    expect(await screen.findByRole("link", { name: "Inbox, 1 waiting" })).toBeTruthy();
    expect(screen.getByRole("link", { name: "Inbox, 1 waiting" }).getAttribute("href")).toBe(`/w/${WS}/inbox`);
  });
});

describe("the switcher", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    fakeApi();
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  test("lists the workspace's projects, and the current project's scopes, Project-wide first and its parts oldest first", async () => {
    renderAt(`${P}/tracks`);
    await pills("Monitoring platform");
    const switcher = await openSwitcher();

    const projects = within(switcher).getByRole("group", { name: "Projects" });
    await waitFor(() =>
      expect(within(projects).getAllByRole("link").map((row) => row.textContent)).toEqual([
        "Monitoring platform2 parts",
        "Installer app1 part",
      ]),
    );
    const scopes = within(switcher).getByRole("group", { name: "Monitoring platform" });
    expect(within(scopes).getAllByRole("link").map((row) => row.textContent)).toEqual([
      "Project-wideall parts",
      "WWisionapp",
      "Iiot-serviceservice",
      "New part",
    ]);
  });

  test("keeps the tab: on a part's Tracks, every scope opens its own Tracks, and the footer says so", async () => {
    renderAt(`${P}/parts/${WISION}/tracks`);
    await pills("Wision");
    const switcher = await openSwitcher();
    const scopes = within(switcher).getByRole("group", { name: "Monitoring platform" });
    const row = (name: RegExp) => within(scopes).getByRole("link", { name });

    await waitFor(() => expect(row(/iot-service/).getAttribute("href")).toBe(`${P}/parts/${IOT}/tracks`));
    expect(row(/Project-wide/).getAttribute("href")).toBe(`${P}/tracks`);
    expect(row(/Wision/).getAttribute("aria-current")).not.toBeNull();
    expect(within(switcher).getByText(/You stay on/).textContent).toBe("You stay on Tracks");
  });

  test("a part's Features opens a project's Parts, and a project's Parts a part's Features", async () => {
    renderAt(`${P}/parts`);
    await pills("Monitoring platform");
    const switcher = await openSwitcher();

    await waitFor(() =>
      expect(within(switcher).getByRole("link", { name: /Wision/ }).getAttribute("href")).toBe(`${P}/parts/${WISION}/features`),
    );
  });

  test("from a track page, the parts open their Tracks", async () => {
    renderAt(`${P}/tracks/${LONG_WINDOWS}/how`);
    await pills("Wision");
    const switcher = await openSwitcher();

    await waitFor(() =>
      expect(within(switcher).getByRole("link", { name: /iot-service/ }).getAttribute("href")).toBe(`${P}/parts/${IOT}/tracks`),
    );
  });

  test("↓ from the find field goes to the current scope, ← to the projects, and Esc closes it", async () => {
    renderAt(`${P}/parts/${IOT}`);
    await pills("iot-service");
    const switcher = await openSwitcher();
    const find = within(switcher).getByRole("textbox", { name: "Find a project or part" });
    await waitFor(() => expect(within(switcher).getByRole("link", { name: /iot-service/ })).toBeTruthy());

    fireEvent.keyDown(find, { key: "ArrowDown" });
    expect(document.activeElement?.textContent).toBe("Iiot-serviceservice");
    fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft" });
    expect(document.activeElement?.textContent).toMatch(/^Monitoring platform/);
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Switch project or part" })).toBeNull();
  });

  test("typing narrows both columns to the names (and kinds) that hold it", async () => {
    renderAt(`${P}`);
    await pills("Monitoring platform");
    const switcher = await openSwitcher();
    await waitFor(() => expect(within(switcher).getByRole("link", { name: /iot-service/ })).toBeTruthy());

    fireEvent.change(within(switcher).getByRole("textbox", { name: "Find a project or part" }), { target: { value: "SERVICE" } });

    expect(within(switcher).getAllByRole("link").map((row) => row.textContent)).toEqual(["Iiot-serviceservice"]);
    expect(within(switcher).getByText("No project matches")).toBeTruthy();
  });
});

describe("old URLs", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    fakeApi();
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  test.each([
    ["the App page", `${P}/app`, P],
    ["a project's Inbox", `${P}/inbox`, `/w/${WS}/inbox`],
    ["a project's My tracks", `${P}/my-tracks`, `/w/${WS}/my-tracks`],
    ["Threads narrowed to a track", `${P}/threads?scope=${LONG_WINDOWS}`, `${P}/tracks/${LONG_WINDOWS}/threads`],
    ["a track's thread", `${P}/threads/${THREAD}`, `${P}/tracks/${LONG_WINDOWS}/threads/${THREAD}`],
    ["a track's doc", `${P}/docs/${TRACK_DOC}`, `${P}/tracks/${LONG_WINDOWS}/docs/${TRACK_DOC}`],
  ])("%s leads to its new place", async (_, old, now) => {
    const router = renderAt(old);
    await waitFor(() => expect(router.state.location.pathname).toBe(now));
  });

  test("a doc link keeps where in the doc to be on its way to the new place", async () => {
    const router = renderAt(`${P}/docs/${TRACK_DOC}?node=p0000001`);
    await waitFor(() => expect(router.state.location.pathname).toBe(`${P}/tracks/${LONG_WINDOWS}/docs/${TRACK_DOC}`));
    expect(router.state.location.search).toEqual({ node: "p0000001" });
  });
});
