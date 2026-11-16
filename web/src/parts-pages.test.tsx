// @vitest-environment jsdom
// US-306: the project page lists its parts, each with its name and kind, and
// creates, renames and deletes one; a part's page, .../parts/<part>, lists its
// features (none until US-307) and its tracks, the active ones (planning and
// building) first, then the done and dropped ones, each group with the most
// recently updated first, each row with its state. A track created there goes
// on that part. Both pages read GET .../parts and GET .../home.
// US-310: the part page creates, renames and deletes its features; a feature's
// page, .../parts/<part>/features/<feature>, shows its name, description, docs
// and linked tracks; a track's page lists the features it changes, read-only.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const WISION = "0192d7a0-3333-7000-8000-000000000001";
const IOT = "0192d7a0-3333-7000-8000-000000000002";
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
      const path = new URL(url, "http://localhost:3000").pathname;
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

describe("the parts on the project page", () => {
  test("each part shows its name and kind and opens its page, and New part creates one with a name and a required kind", async () => {
    const api = fakeApi({ parts: [part(WISION, "Wision", "app"), part(IOT, "iot-service", "service")] });
    renderAt(`${PROJECT_PATH}/app`);
    const parts = await region("Parts");
    await within(parts).findByText("Wision");
    const links = within(parts).getAllByRole("link");
    expect(links.map(shownText)).toEqual(["Wisionapp", "iot-serviceservice"]);
    expect(links[0].getAttribute("href")).toBe(`${PROJECT_PATH}/parts/${WISION}`);

    fireEvent.click(within(parts).getByRole("button", { name: "New part" }));
    fireEvent.change(within(parts).getByRole("textbox", { name: "Part name" }), { target: { value: "wision-api" } });
    const create = within(parts).getByRole("button", { name: "Create part" }) as HTMLButtonElement;
    expect(create.disabled, "no kind, no create").toBe(true);
    fireEvent.change(within(parts).getByRole("combobox", { name: "Part kind" }), { target: { value: "service" } });
    fireEvent.click(create);

    await waitFor(() =>
      expect(api.writes).toEqual([{ method: "POST", path: `${API}/parts`, body: { name: "wision-api", kind: "service", description: "" } }]),
    );
    await within(parts).findByText("wision-api");
  });

  test("a part is renamed in place, and deleted only once the delete is confirmed", async () => {
    const api = fakeApi({ parts: [part(WISION, "Wision", "app")] });
    renderAt(`${PROJECT_PATH}/app`);
    const parts = await region("Parts");
    await within(parts).findByText("Wision");

    fireEvent.click(within(parts).getByRole("button", { name: "Rename Wision" }));
    fireEvent.change(within(parts).getByRole("textbox", { name: "Part name" }), { target: { value: "Wision web" } });
    fireEvent.click(within(parts).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "PATCH", path: `${API}/parts/${WISION}`, body: { name: "Wision web" } }]));

    api.writes = [];
    fireEvent.click(await within(parts).findByRole("button", { name: "Delete Wision" }));
    expect(api.writes, "asking first sends nothing").toEqual([]);
    fireEvent.click(within(parts).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "DELETE", path: `${API}/parts/${WISION}`, body: null }]));
  });
});

describe("the part page", () => {
  test("lists no features yet, and the part's tracks, active ones then done and dropped, each group most recently updated first, each with its state", async () => {
    fakeApi({
      parts: [part(WISION, "Wision", "app"), part(IOT, "iot-service", "service")],
      tracks: [
        track("Old reports", WISION, "done", "2026-10-21T14:00:00Z"),
        track("Alarms", WISION, "planning", "2026-10-21T10:00:00Z"),
        track("Gateway", IOT, "building", "2026-10-21T13:00:00Z"),
        track("Charts", WISION, "building", "2026-10-21T12:00:00Z"),
        track("Maps", WISION, "dropped", "2026-10-21T15:00:00Z"),
        track("Login", WISION, "planning", null, "2026-10-21T11:00:00Z"),
      ],
    });
    renderAt(`${PROJECT_PATH}/parts/${WISION}`);
    expect(await within(await main()).findByRole("heading", { level: 1, name: "Wision" })).toBeTruthy();

    const features = await region("Features");
    expect(features.textContent).toContain("No features yet");

    const tracks = await region("Tracks");
    await within(tracks).findByText("Alarms");
    const rows = within(tracks).getAllByRole("link").map((row) => [row.querySelector("span span")?.textContent, within(row).getByText(/^(planning|building|done|dropped)$/).textContent]);
    expect(rows).toEqual([
      ["Charts", "building"],
      ["Login", "planning"],
      ["Alarms", "planning"],
      ["Maps", "dropped"],
      ["Old reports", "done"],
    ]);
  });

  test("New feature creates one with a name and a description, a feature is renamed by name only, and deleted once confirmed", async () => {
    const ANALYZE = "0192d7a0-5555-7000-8000-000000000001";
    const FEATURES = `${API}/parts/${WISION}/features`;
    const api = fakeApi({
      parts: [part(WISION, "Wision", "app")],
      features: [feature(ANALYZE, WISION, "Analyze", "Look at long windows of data")],
      nameTakenOnce: FEATURES,
    });
    renderAt(`${PROJECT_PATH}/parts/${WISION}`);
    const features = await region("Features");
    const analyze = await within(features).findByRole("link", { name: /Analyze/ });
    expect(analyze.getAttribute("href")).toBe(`${PROJECT_PATH}/parts/${WISION}/features/${ANALYZE}`);

    fireEvent.click(within(features).getByRole("button", { name: "New feature" }));
    fireEvent.change(within(features).getByRole("textbox", { name: "Feature name" }), { target: { value: "Alarms" } });
    fireEvent.change(within(features).getByRole("textbox", { name: "Feature description" }), { target: { value: "Warn on thresholds" } });
    fireEvent.click(within(features).getByRole("button", { name: "Create feature" }));
    expect(await within(features).findByRole("alert"), "a 409 name_taken is shown").toBeTruthy();
    expect(within(features).getByRole("textbox", { name: "Feature name" }), "and the form stays open").toBeTruthy();
    fireEvent.click(within(features).getByRole("button", { name: "Create feature" }));
    await within(features).findByRole("link", { name: /Alarms/ });
    expect(api.writes.map((w) => [w.method, w.path, w.body])).toEqual([
      ["POST", FEATURES, { name: "Alarms", description: "Warn on thresholds" }],
      ["POST", FEATURES, { name: "Alarms", description: "Warn on thresholds" }],
    ]);

    api.writes = [];
    fireEvent.click(within(features).getByRole("button", { name: "Rename Analyze" }));
    expect(within(features).queryByRole("textbox", { name: "Feature description" }), "no description editing").toBeNull();
    fireEvent.change(within(features).getByRole("textbox", { name: "Feature name" }), { target: { value: "Analyse" } });
    fireEvent.click(within(features).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "PATCH", path: `${FEATURES}/${ANALYZE}`, body: { name: "Analyse" } }]));

    api.writes = [];
    fireEvent.click(await within(features).findByRole("button", { name: "Delete Analyse" }));
    expect(api.writes, "asking first sends nothing").toEqual([]);
    fireEvent.click(within(features).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "DELETE", path: `${FEATURES}/${ANALYZE}`, body: null }]));
    await waitFor(() => expect(within(features).queryByRole("link", { name: /Analyse/ })).toBeNull());
  });

  test("New track puts the track on the page's part without asking for one", async () => {
    const api = fakeApi({ parts: [part(WISION, "Wision", "app"), part(IOT, "iot-service", "service")] });
    renderAt(`${PROJECT_PATH}/parts/${IOT}`);
    const tracks = await region("Tracks");
    fireEvent.click(within(tracks).getByRole("button", { name: "New track" }));
    expect(within(tracks).queryByRole("combobox", { name: "Part" })).toBeNull();
    fireEvent.change(within(tracks).getByRole("textbox", { name: "Track name" }), { target: { value: "Gateway" } });
    fireEvent.click(within(tracks).getByRole("button", { name: "Create track" }));
    await waitFor(() =>
      expect(api.writes).toEqual([{ method: "POST", path: `${API}/tracks`, body: { name: "Gateway", description: "", part_id: IOT } }]),
    );
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
