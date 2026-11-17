// @vitest-environment jsdom
// The project's Parts (.../parts) and a part's Features (.../parts/<part>/features)
// as built in slice S2 (prototype tasks/ui-redesign/v2/parts.html,
// part-wision-features.html): a row per part with its kind, what it is, its
// features and what is in flight on it, a card per feature with what it does
// and the track changing it; creating, renaming and deleting both (US-306,
// US-310). Then the feature page and the features on a track page (slice S3's
// screens), which read GET .../parts and GET .../home.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import * as scope from "./scope-pages-fake";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const WISION = "0192d7a0-3333-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T08:00:00Z";

type Call = { method: string; path: string; body: unknown };

function part(id: string, name: string, kind: string) {
  return { id, project_id: PROJECT_ID, name, kind, description: "", created_at: AT };
}

let counter = 0;

function track(name: string, partId: string, state: string, lastActivity: string | null, createdAt = AT) {
  counter += 1;
  return {
    track: {
      id: `0192d7a0-4444-7000-8000-${counter.toString(16).padStart(12, "0")}`,
      project_id: PROJECT_ID,
      part_id: partId,
      state,
      name,
      description: "",
      created_at: createdAt,
    },
    docs: 0,
    waiting: 0,
    stories: { total: 0, open: 0, done: 0 },
    last_activity: lastActivity,
    part_name: "",
    part_deleted: false,
  };
}

type Track = ReturnType<typeof track>;

function feature(id: string, partId: string, name: string, description = "") {
  return { id, project_id: PROJECT_ID, part_id: partId, name, description, created_at: AT };
}

type Feature = ReturnType<typeof feature>;

function doc(id: string, featureId: string, kind: string, title: string) {
  return { id, project_id: PROJECT_ID, track_id: null, feature_id: featureId, kind, pillar: "spec", title, created_at: AT };
}

/** A fake of the API that keeps the parts and features, and records every write. */
function fakeApi(fields: {
  parts: ReturnType<typeof part>[];
  tracks?: Track[];
  features?: Feature[];
  /** The docs answered for GET .../docs?feature=<id>. */
  featureDocs?: ReturnType<typeof doc>[];
  /** The tracks answered for GET .../features/<id>/tracks. */
  featureTracks?: Track["track"][];
  /** The features answered for GET .../tracks/<id>/features. */
  trackFeatures?: Feature[];
  /** A write path that answers 409 name_taken once. */
  nameTakenOnce?: string;
}) {
  const api = { parts: fields.parts, features: fields.features ?? [], writes: [] as Call[] };
  let nameTaken = fields.nameTakenOnce;
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const { pathname: path } = new URL(url, "http://localhost:3000");
      const method = (init?.method ?? "GET").toUpperCase();
      const featureAt = path.match(/\/parts\/([^/]+)\/features(?:\/([^/]+))?(\/tracks)?$/);
      if (method !== "GET") {
        const body = init?.body ? JSON.parse(String(init.body)) : null;
        api.writes.push({ method, path, body });
        if (nameTaken === path) {
          nameTaken = undefined;
          return json(409, { error: "name_taken" });
        }
        if (featureAt && method === "POST") {
          const created = feature("0192d7a0-5555-7000-8000-0000000000ff", featureAt[1], body.name, body.description);
          api.features = [...api.features, created];
          return json(201, { feature: created });
        }
        if (featureAt && method === "PATCH") {
          api.features = api.features.map((f) => (f.id === featureAt[2] ? { ...f, name: body.name } : f));
          return json(200, { feature: api.features.find((f) => f.id === featureAt[2]) });
        }
        if (featureAt && method === "DELETE") {
          api.features = api.features.filter((f) => f.id !== featureAt[2]);
          return new Response(null, { status: 204 });
        }
        if (method === "POST" && path === `${API}/parts`) {
          const created = part("0192d7a0-3333-7000-8000-0000000000ff", body.name, body.kind);
          api.parts = [...api.parts, created];
          return json(201, { part: created });
        }
        if (method === "POST" && path === `${API}/tracks`)
          return json(201, { track: { ...track(body.name, body.part_id, "planning", null).track } });
        if (method === "PATCH") return json(200, { part: { ...api.parts[0], name: body.name } });
        return new Response(null, { status: 204 });
      }
      if (featureAt?.[3]) return json(200, { tracks: fields.featureTracks ?? [] });
      if (featureAt?.[2]) {
        const one = api.features.find((f) => f.id === featureAt[2]);
        return one ? json(200, { feature: one }) : json(404, { error: "not_found" });
      }
      if (featureAt) return json(200, { features: api.features.filter((f) => f.part_id === featureAt[1]) });
      const trackAt = path.match(/\/tracks\/([^/]+)(\/features)?$/);
      if (trackAt?.[2]) return json(200, { features: fields.trackFeatures ?? [] });
      if (trackAt) {
        const one = (fields.tracks ?? []).find((summary) => summary.track.id === trackAt[1]);
        return one ? json(200, { track: one.track }) : json(404, { error: "not_found" });
      }
      switch (path) {
        case "/api/v1/auth/me":
          return json(200, { user: USER });
        case "/api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `/api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Monitoring platform", created_at: AT }] });
        case `${API}/parts`:
          return json(200, { parts: api.parts });
        case `${API}/home`:
          return json(200, {
            project_id: PROJECT_ID,
            app: { last_activity: null },
            tracks: fields.tracks ?? [],
            docs: [],
            threads: [],
            waiting: { comments: [] },
          });
        case `${API}/docs`:
          return json(200, { docs: fields.featureDocs ?? [] });
        case `${API}/tracks`:
          return json(200, { tracks: (fields.tracks ?? []).map((summary) => summary.track) });
        case `${API}/comments`:
          return json(200, { project_id: PROJECT_ID, comments: [] });
        default:
          return json(404, { error: "not_found" });
      }
    }),
  );
  return api;
}

/** A WebSocket that never opens: these pages are not tested live here. */
class FakeWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  readyState = FakeWebSocket.CONNECTING;
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
  render(<RouterProvider router={router} />);
  return router;
}

/** The text of `element` without its decoration (what is aria-hidden or only for screen readers). */
function shownText(element: HTMLElement) {
  const copy = element.cloneNode(true) as HTMLElement;
  for (const hidden of Array.from(copy.querySelectorAll("[aria-hidden], .sr-only"))) hidden.remove();
  return (copy.textContent ?? "").trim();
}

const main = () => screen.findByRole("main");
const region = async (name: string) => within(await main()).findByRole("region", { name });

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the project's Parts", () => {
  test("a row per part shows its name, kind, what it is, how many features it has and what is in flight on it, and opens the part", async () => {
    const wision = scope.part("Wision", "app", { description: "The web app customers log in to.\nRuns on the CDN." });
    const iot = scope.part("iot-service", "service", { description: "Ingests and stores readings." });
    const api = scope.part("wision-api", "service");
    scope.fakeScopeApi({
      parts: [wision, iot, api],
      features: [scope.feature(wision.id, "Dashboards"), scope.feature(wision.id, "Analyze"), scope.feature(iot.id, "Ingest")],
      tracks: [
        scope.track("Long windows", wision.id, "planning"),
        scope.track("Exports", wision.id, "planning"),
        scope.track("Query", iot.id, "building"),
        scope.track("Charts", wision.id, "done"),
        scope.track("Moves", api.id, "dropped"),
      ],
    });
    scope.renderAt(`${scope.PROJECT_PATH}/parts`);
    const list = await within(await main()).findByRole("list", { name: "Parts" });
    await waitFor(() => expect(within(list).getAllByRole("listitem").map(scope.shownText)[0]).toContain("2 features"));
    expect(within(list).getAllByRole("listitem").map(scope.shownText)).toEqual([
      "Wision app The web app customers log in to. 2 features 2 planning",
      "iot-service service Ingests and stores readings. 1 feature 1 building",
      "wision-api service 0 features Nothing in flight",
    ]);
    expect(within(list).getByRole("link", { name: "Wision" }).getAttribute("href")).toBe(`${scope.PROJECT_PATH}/parts/${wision.id}`);
  });

  test("Create part needs a name and a kind, which starts empty; a taken name is said and the form stays; the new part is listed", async () => {
    const api = scope.fakeScopeApi({ parts: [scope.part("Wision", "app")] });
    scope.renderAt(`${scope.PROJECT_PATH}/parts`);
    const page = await main();
    fireEvent.click(await within(page).findByRole("button", { name: "Create part" }));
    const form = within(page).getByRole("form", { name: "Create part" });
    fireEvent.change(within(form).getByRole("textbox", { name: "Part name" }), { target: { value: "wision-api" } });
    const create = within(form).getByRole("button", { name: "Create part" }) as HTMLButtonElement;
    expect((within(form).getByRole("combobox", { name: "Part kind" }) as HTMLSelectElement).value).toBe("");
    expect(create.disabled, "no kind, no create").toBe(true);
    fireEvent.change(within(form).getByRole("combobox", { name: "Part kind" }), { target: { value: "service" } });
    fireEvent.change(within(form).getByRole("textbox", { name: "Part description" }), { target: { value: " Users and roles " } });
    fireEvent.click(create);

    await waitFor(() => expect(api.writes).toEqual([{ method: "POST", path: `${scope.API}/parts`, body: { name: "wision-api", kind: "service", description: "Users and roles" } }]));
    const list = within(page).getByRole("list", { name: "Parts" });
    expect(await within(list).findByRole("link", { name: "wision-api" })).toBeTruthy();
    expect(within(page).queryByRole("form", { name: "Create part" }), "the form closes").toBeNull();
  });

  test("a part is renamed in place, and deleted only once the delete is confirmed", async () => {
    const wision = scope.part("Wision", "app");
    const api = scope.fakeScopeApi({ parts: [wision] });
    scope.renderAt(`${scope.PROJECT_PATH}/parts`);
    const list = await within(await main()).findByRole("list", { name: "Parts" });
    fireEvent.click(await within(list).findByRole("button", { name: "Rename Wision" }));
    fireEvent.change(within(list).getByRole("textbox", { name: "Part name" }), { target: { value: "Wision web" } });
    fireEvent.click(within(list).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "PATCH", path: `${scope.API}/parts/${wision.id}`, body: { name: "Wision web" } }]));

    api.writes.length = 0;
    fireEvent.click(await within(list).findByRole("button", { name: "Delete Wision web" }));
    expect(api.writes, "asking first sends nothing").toEqual([]);
    fireEvent.click(within(list).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "DELETE", path: `${scope.API}/parts/${wision.id}`, body: null }]));
    await waitFor(() => expect(within(list).queryByRole("link", { name: "Wision web" })).toBeNull());
  });
});

describe("a part's Features", () => {
  test("a card per feature says what it does, the track in flight changing it or that nothing is, and how many decisions and docs it has", async () => {
    const wision = scope.part("Wision", "app");
    const dashboards = scope.feature(wision.id, "Dashboards", "Widgets for the latest readings.");
    const analyze = scope.feature(wision.id, "Analyze", "Time-series chart per channel.");
    const windows = scope.track("Long time windows", wision.id, "planning");
    const charts = scope.track("First chart", wision.id, "done");
    const raw = scope.decision("Readings are always raw", charts.track.id, { feature: analyze, part: wision }, { at: "2026-03-01T08:00:00Z" });
    const hourly = scope.decision("Aggregate per hour", windows.track.id, { feature: analyze, part: wision }, { replaces: raw.id });
    raw.replaced_by_decision_id = hourly.id;
    scope.fakeScopeApi({
      parts: [wision],
      features: [dashboards, analyze],
      tracks: [windows, charts],
      changes: { [analyze.id]: [windows.track.id, charts.track.id], [dashboards.id]: [charts.track.id] },
      decisions: [raw, hourly, scope.decision("Widgets refresh once a minute", charts.track.id, { feature: dashboards, part: wision })],
      docs: [scope.doc("How compare works", "architecture", { feature: analyze.id })],
    });
    scope.renderAt(`${scope.PROJECT_PATH}/parts/${wision.id}/features`);
    const list = await within(await main()).findByRole("list", { name: "Features" });
    await waitFor(() => expect(within(list).getAllByRole("listitem").map(scope.shownText)[1]).toContain("1 replaced"));
    expect(within(list).getAllByRole("listitem").map(scope.shownText)).toEqual([
      "Dashboards Widgets for the latest readings. Nothing in flight · added 10 Nov 1 decision 0 docs",
      "Analyze Time-series chart per channel. Being changed by Long time windows 1 decision, 1 replaced 1 doc",
    ]);
    expect(within(list).getByRole("link", { name: "Long time windows" }).getAttribute("href")).toBe(`${scope.PROJECT_PATH}/tracks/${windows.track.id}`);
    expect(within(list).getByRole("link", { name: "Analyze" }).getAttribute("href")).toBe(`${scope.PROJECT_PATH}/parts/${wision.id}/features/${analyze.id}`);
  });

  test("Create feature takes a name and what it does; a feature is renamed by name only, and deleted once confirmed", async () => {
    const wision = scope.part("Wision", "app");
    const analyze = scope.feature(wision.id, "Analyze", "Look at long windows of data");
    const FEATURES = `${scope.API}/parts/${wision.id}/features`;
    const api = scope.fakeScopeApi({ parts: [wision], features: [analyze] });
    scope.renderAt(`${scope.PROJECT_PATH}/parts/${wision.id}/features`);
    const page = await main();
    fireEvent.click(await within(page).findByRole("button", { name: "Create feature" }));
    const form = within(page).getByRole("form", { name: "Create feature" });
    fireEvent.change(within(form).getByRole("textbox", { name: "Feature name" }), { target: { value: "Alarms" } });
    fireEvent.change(within(form).getByRole("textbox", { name: "Feature description" }), { target: { value: "Warn on thresholds" } });
    fireEvent.click(within(form).getByRole("button", { name: "Create feature" }));
    const list = within(page).getByRole("list", { name: "Features" });
    await within(list).findByRole("link", { name: "Alarms" });
    expect(api.writes).toEqual([{ method: "POST", path: FEATURES, body: { name: "Alarms", description: "Warn on thresholds" } }]);

    api.writes.length = 0;
    fireEvent.click(within(list).getByRole("button", { name: "Rename Analyze" }));
    expect(within(list).queryByRole("textbox", { name: "Feature description" }), "no description editing").toBeNull();
    fireEvent.change(within(list).getByRole("textbox", { name: "Feature name" }), { target: { value: "Analyse" } });
    fireEvent.click(within(list).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "PATCH", path: `${FEATURES}/${analyze.id}`, body: { name: "Analyse" } }]));

    api.writes.length = 0;
    fireEvent.click(await within(list).findByRole("button", { name: "Delete Analyse" }));
    expect(api.writes, "asking first sends nothing").toEqual([]);
    fireEvent.click(within(list).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "DELETE", path: `${FEATURES}/${analyze.id}`, body: null }]));
    await waitFor(() => expect(within(list).queryByRole("link", { name: "Analyse" })).toBeNull());
  });
});

describe("the feature page", () => {
  const ANALYZE = "0192d7a0-5555-7000-8000-000000000001";
  const FEATURE_PATH = `${PROJECT_PATH}/parts/${WISION}/features/${ANALYZE}`;

  test("shows the feature's name, description and docs, each opening the doc viewer, and its linked tracks, active first, each with its state, opening the track page", async () => {
    const DOC = "0192d7a0-6666-7000-8000-000000000001";
    const tracks = [
      track("Old charts", WISION, "done", "2026-10-21T14:00:00Z"),
      track("Long windows", WISION, "building", "2026-10-21T12:00:00Z"),
      track("Exports", WISION, "planning", "2026-10-21T13:00:00Z"),
    ];
    fakeApi({
      parts: [part(WISION, "Wision", "app")],
      tracks,
      features: [feature(ANALYZE, WISION, "Analyze", "Look at long windows of data")],
      featureDocs: [doc(DOC, ANALYZE, "html", "Analyze overview")],
      featureTracks: tracks.map((summary) => summary.track),
    });
    renderAt(FEATURE_PATH);
    const page = await main();
    expect(await within(page).findByRole("heading", { level: 1, name: "Analyze" })).toBeTruthy();
    expect(page.textContent).toContain("Look at long windows of data");

    const docLink = await within(page).findByRole("link", { name: "Analyze overview" });
    expect(docLink.getAttribute("href")).toBe(`${PROJECT_PATH}/docs/${DOC}`);

    const linked = await region("Tracks");
    await within(linked).findByText("Long windows");
    const rows = within(linked)
      .getAllByRole("link")
      .map((row) => [row.querySelector("span span")?.textContent, within(row).getByText(/^(planning|building|done|dropped)$/).textContent, row.getAttribute("href")]);
    expect(rows).toEqual([
      ["Exports", "planning", `${PROJECT_PATH}/tracks/${tracks[2].track.id}`],
      ["Long windows", "building", `${PROJECT_PATH}/tracks/${tracks[1].track.id}`],
      ["Old charts", "done", `${PROJECT_PATH}/tracks/${tracks[0].track.id}`],
    ]);
  });
});

describe("the features on a track page", () => {
  test("lists the features the track changes, each opening its feature page, with no way to link or unlink one", async () => {
    const ANALYZE = "0192d7a0-5555-7000-8000-000000000001";
    const ALARMS = "0192d7a0-5555-7000-8000-000000000002";
    const windows = track("Long windows", WISION, "building", "2026-10-21T12:00:00Z");
    windows.part_name = "Wision";
    const api = fakeApi({
      parts: [part(WISION, "Wision", "app")],
      tracks: [windows],
      trackFeatures: [feature(ANALYZE, WISION, "Analyze"), feature(ALARMS, WISION, "Alarms")],
    });
    renderAt(`${PROJECT_PATH}/tracks/${windows.track.id}`);
    const features = await region("Features");
    await within(features).findByText("Analyze");
    expect(within(features).getAllByRole("link").map((link) => [shownText(link), link.getAttribute("href")])).toEqual([
      ["Analyze", `${PROJECT_PATH}/parts/${WISION}/features/${ANALYZE}`],
      ["Alarms", `${PROJECT_PATH}/parts/${WISION}/features/${ALARMS}`],
    ]);
    expect(within(features).queryAllByRole("button"), "read-only: linking stays in the CLI").toEqual([]);
    expect(api.writes).toEqual([]);
  });
});
